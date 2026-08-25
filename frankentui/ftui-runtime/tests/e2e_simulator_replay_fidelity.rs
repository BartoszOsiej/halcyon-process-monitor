#![forbid(unsafe_code)]

//! bd-37a.10: E2E test — Deterministic simulator replay fidelity.
//!
//! Verifies that recording + replaying through the FrankenLab infrastructure
//! produces byte-identical terminal output and model state for all event types.
//!
//! Scenarios:
//! 1. Simple text input
//! 2. Window resize sequence
//! 3. Rapid keystroke burst
//! 4. Mouse event sequence
//! 5. Animation sequence with time injection
//!
//! Run:
//!   cargo test -p ftui-runtime --test e2e_simulator_replay_fidelity

use ftui_core::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, Modifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ftui_harness::lab_integration::{Lab, LabConfig, LabSession, Recording};
use ftui_render::cell::Cell;
use ftui_render::frame::Frame;
use ftui_runtime::program::{Cmd, Model};

// ============================================================================
// Test Model: A simple application that responds to all event types
// ============================================================================

#[derive(Clone)]
struct ReplayTestModel {
    /// Counter incremented by various interactions.
    counter: i32,
    /// Last key pressed (char code).
    last_key: Option<char>,
    /// Current viewport dimensions.
    viewport: (u16, u16),
    /// Mouse click positions.
    clicks: Vec<(u16, u16)>,
    /// Tick counter for animation.
    tick_count: u64,
    /// Whether quit was requested.
    quit: bool,
}

impl ReplayTestModel {
    fn new() -> Self {
        Self {
            counter: 0,
            last_key: None,
            viewport: (80, 24),
            clicks: Vec::new(),
            tick_count: 0,
            quit: false,
        }
    }
}

#[derive(Debug)]
enum TestMsg {
    KeyPress(char),
    Resize(u16, u16),
    Click(u16, u16),
    Tick,
    Quit,
    Other,
}

impl From<Event> for TestMsg {
    fn from(event: Event) -> Self {
        match event {
            Event::Key(k) => match k.code {
                KeyCode::Char('q') => TestMsg::Quit,
                KeyCode::Char(c) => TestMsg::KeyPress(c),
                _ => TestMsg::Other,
            },
            Event::Resize { width, height } => TestMsg::Resize(width, height),
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Down(MouseButton::Left) => TestMsg::Click(m.x, m.y),
                _ => TestMsg::Other,
            },
            Event::Tick => TestMsg::Tick,
            _ => TestMsg::Other,
        }
    }
}

impl Model for ReplayTestModel {
    type Message = TestMsg;

    fn init(&mut self) -> Cmd<Self::Message> {
        self.counter = 0;
        Cmd::none()
    }

    fn update(&mut self, msg: Self::Message) -> Cmd<Self::Message> {
        match msg {
            TestMsg::KeyPress(c) => {
                self.counter += 1;
                self.last_key = Some(c);
                Cmd::none()
            }
            TestMsg::Resize(w, h) => {
                self.viewport = (w, h);
                Cmd::none()
            }
            TestMsg::Click(x, y) => {
                self.clicks.push((x, y));
                self.counter += 1;
                Cmd::none()
            }
            TestMsg::Tick => {
                self.tick_count += 1;
                Cmd::none()
            }
            TestMsg::Quit => {
                self.quit = true;
                Cmd::quit()
            }
            TestMsg::Other => Cmd::none(),
        }
    }

    fn view(&self, frame: &mut Frame) {
        // Render counter and state into the buffer deterministically.
        let text = format!(
            "c={} k={} v={}x{} t={} cl={}",
            self.counter,
            self.last_key.unwrap_or('_'),
            self.viewport.0,
            self.viewport.1,
            self.tick_count,
            self.clicks.len()
        );
        for (i, c) in text.chars().enumerate() {
            if (i as u16) < frame.width() {
                frame.buffer.set_raw(i as u16, 0, Cell::from_char(c));
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn key(c: char) -> Event {
    Event::Key(KeyEvent {
        code: KeyCode::Char(c),
        modifiers: Modifiers::empty(),
        kind: KeyEventKind::Press,
    })
}

fn mouse_click(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent::new(
        MouseEventKind::Down(MouseButton::Left),
        x,
        y,
    ))
}

fn resize(w: u16, h: u16) -> Event {
    Event::Resize {
        width: w,
        height: h,
    }
}

/// Record a scenario and then replay it, asserting replay matches.
fn record_and_replay(
    name: &str,
    seed: u64,
    scenario: impl Fn(&mut LabSession<ReplayTestModel>),
) -> Recording {
    Lab::assert_replay_deterministic(
        LabConfig::new("e2e_replay", name, seed)
            .viewport(80, 24)
            .time_step_ms(16)
            .log_frame_checksums(true),
        ReplayTestModel::new,
        scenario,
    )
}

// ============================================================================
// 1. Simple text input
// ============================================================================

#[test]
fn replay_simple_text_input() {
    let recording = record_and_replay("simple_text", 1001, |s| {
        s.init();
        s.inject_event(key('h'));
        s.tick();
        s.capture_frame();
        s.inject_event(key('e'));
        s.tick();
        s.capture_frame();
        s.inject_event(key('l'));
        s.tick();
        s.capture_frame();
        s.inject_event(key('l'));
        s.tick();
        s.capture_frame();
        s.inject_event(key('o'));
        s.tick();
        s.capture_frame();
    });

    assert_eq!(recording.frame_records.len(), 5);
    // All frames should have non-zero checksums
    for fr in &recording.frame_records {
        assert_ne!(fr.checksum, 0, "Frame checksum should be non-zero");
    }
}

/// Replay with different seed should still produce identical results
/// (seed affects LabScenario scheduling, not model logic).
#[test]
fn replay_text_input_100_seeds() {
    for seed in 0..100 {
        let _recording = record_and_replay(&format!("text_seed_{seed}"), seed, |s| {
            s.init();
            s.inject_event(key('a'));
            s.tick();
            s.capture_frame();
        });
    }
}

// ============================================================================
// 2. Window resize sequence
// ============================================================================

#[test]
fn replay_resize_sequence() {
    let recording = record_and_replay("resize_sequence", 2001, |s| {
        s.init();
        s.tick();
        s.capture_frame();

        // Resize to larger
        s.inject_event(resize(120, 40));
        s.tick();
        s.capture_frame();

        // Resize to smaller
        s.inject_event(resize(40, 12));
        s.tick();
        s.capture_frame();

        // Resize back to original
        s.inject_event(resize(80, 24));
        s.tick();
        s.capture_frame();
    });

    assert_eq!(recording.frame_records.len(), 4);
}

/// Multiple rapid resizes then capture — only final state matters.
#[test]
fn replay_rapid_resize_burst() {
    let recording = record_and_replay("rapid_resize", 2002, |s| {
        s.init();

        // Rapid resize burst without ticks between
        for w in 80..=100 {
            s.inject_event(resize(w, 24));
        }
        s.tick();
        s.capture_frame();

        // Model should reflect the last resize
        assert_eq!(
            s.model().viewport,
            (100, 24),
            "Model should reflect final resize"
        );
    });

    assert_eq!(recording.frame_records.len(), 1);
}

// ============================================================================
// 3. Rapid keystroke burst
// ============================================================================

#[test]
fn replay_rapid_keystroke_burst() {
    let recording = record_and_replay("keystroke_burst", 3001, |s| {
        s.init();

        // Type 20 characters rapidly (avoiding 'q' which triggers quit)
        for c in "abcdefghijklmnoprstu".chars() {
            s.inject_event(key(c));
        }
        s.tick();
        s.capture_frame();

        assert_eq!(s.model().counter, 20, "All 20 keystrokes should count");
        assert_eq!(s.model().last_key, Some('u'));
    });

    assert_eq!(recording.frame_records.len(), 1);
    assert_ne!(recording.frame_records[0].checksum, 0);
}

/// Burst with interleaved ticks and frame captures.
#[test]
fn replay_keystroke_burst_with_captures() {
    let recording = record_and_replay("keystroke_captures", 3002, |s| {
        s.init();

        for (i, c) in "abcdefghij".chars().enumerate() {
            s.inject_event(key(c));
            s.tick();
            if i % 3 == 0 {
                s.capture_frame();
            }
        }
    });

    // Captures at indices 0, 3, 6, 9 → 4 frames
    assert_eq!(recording.frame_records.len(), 4);

    // Each frame should be different (counter changes)
    let checksums: Vec<u64> = recording.frame_records.iter().map(|f| f.checksum).collect();
    for i in 1..checksums.len() {
        assert_ne!(
            checksums[i],
            checksums[i - 1],
            "Consecutive frames should differ"
        );
    }
}

// ============================================================================
// 4. Mouse event sequence
// ============================================================================

#[test]
fn replay_mouse_event_sequence() {
    let recording = record_and_replay("mouse_sequence", 4001, |s| {
        s.init();

        // Click at various positions
        s.inject_event(mouse_click(10, 5));
        s.tick();
        s.capture_frame();

        s.inject_event(mouse_click(20, 10));
        s.tick();
        s.capture_frame();

        s.inject_event(mouse_click(0, 0));
        s.tick();
        s.capture_frame();

        assert_eq!(s.model().clicks.len(), 3);
        assert_eq!(s.model().clicks[0], (10, 5));
        assert_eq!(s.model().clicks[1], (20, 10));
        assert_eq!(s.model().clicks[2], (0, 0));
    });

    assert_eq!(recording.frame_records.len(), 3);
}

/// Mouse clicks interleaved with keyboard input.
#[test]
fn replay_mixed_mouse_and_keys() {
    let recording = record_and_replay("mixed_input", 4002, |s| {
        s.init();

        s.inject_event(key('x'));
        s.inject_event(mouse_click(5, 5));
        s.inject_event(key('y'));
        s.inject_event(mouse_click(15, 15));
        s.tick();
        s.capture_frame();

        assert_eq!(s.model().counter, 4); // 2 keys + 2 clicks
        assert_eq!(s.model().clicks.len(), 2);
        assert_eq!(s.model().last_key, Some('y'));
    });

    assert_eq!(recording.frame_records.len(), 1);
}

// ============================================================================
// 5. Animation sequence with time injection
// ============================================================================

#[test]
fn replay_animation_with_ticks() {
    let recording = record_and_replay("animation_ticks", 5001, |s| {
        s.init();

        // Simulate 20 frames of animation
        for _ in 0..20 {
            s.tick();
            s.capture_frame();
        }

        assert_eq!(s.model().tick_count, 20);
    });

    assert_eq!(recording.frame_records.len(), 20);

    // All 20 frames should have the same checksum pattern
    // (tick_count changes view, so each frame should differ)
    let unique_checksums: std::collections::HashSet<u64> =
        recording.frame_records.iter().map(|f| f.checksum).collect();
    assert!(
        unique_checksums.len() > 1,
        "Animation frames should produce varying checksums"
    );
}

/// Animation with injected events between frames.
#[test]
fn replay_animation_with_events_between_frames() {
    let recording = record_and_replay("animation_events", 5002, |s| {
        s.init();

        // 10 frames, inject a key every 3rd frame
        for i in 0..10 {
            if i % 3 == 0 {
                s.inject_event(key('a'));
            }
            s.tick();
            s.capture_frame();
        }

        // Keys injected at frames 0, 3, 6, 9 → 4 keys
        assert_eq!(s.model().counter, 4);
        assert_eq!(s.model().tick_count, 10);
    });

    assert_eq!(recording.frame_records.len(), 10);
}

// ============================================================================
// Cross-scenario: complex combined workflow
// ============================================================================

/// Full workflow: init → keys → resize → mouse → animation → verify replay.
#[test]
fn replay_full_combined_workflow() {
    let recording = record_and_replay("full_workflow", 9001, |s| {
        s.init();

        // Phase 1: Type some text
        for c in "hello".chars() {
            s.inject_event(key(c));
        }
        s.tick();
        s.capture_frame();

        // Phase 2: Resize
        s.inject_event(resize(100, 40));
        s.tick();
        s.capture_frame();

        // Phase 3: Mouse clicks
        s.inject_event(mouse_click(10, 10));
        s.inject_event(mouse_click(20, 20));
        s.tick();
        s.capture_frame();

        // Phase 4: Animation (5 frames)
        for _ in 0..5 {
            s.tick();
            s.capture_frame();
        }

        // Phase 5: More input + resize
        s.inject_event(key('z'));
        s.inject_event(resize(80, 24));
        s.tick();
        s.capture_frame();

        // Verify final model state
        assert_eq!(s.model().counter, 8); // 5 keys + 2 clicks + 1 key
        assert_eq!(s.model().viewport, (80, 24));
        assert_eq!(s.model().clicks.len(), 2);
        assert_eq!(s.model().last_key, Some('z'));
    });

    // 1 + 1 + 1 + 5 + 1 = 9 frames
    assert_eq!(recording.frame_records.len(), 9);
}

// ============================================================================
// Replay correctness: explicit recording + manual replay comparison
// ============================================================================

/// Manually record and replay, comparing frame checksums one by one.
#[test]
fn explicit_record_replay_frame_comparison() {
    let config = LabConfig::new("e2e_replay", "explicit_compare", 7777)
        .viewport(80, 24)
        .time_step_ms(16)
        .log_frame_checksums(true);

    let scenario = |s: &mut LabSession<ReplayTestModel>| {
        s.init();
        s.inject_event(key('a'));
        s.tick();
        s.capture_frame();
        s.inject_event(key('b'));
        s.tick();
        s.capture_frame();
        s.inject_event(resize(100, 30));
        s.tick();
        s.capture_frame();
        s.inject_event(mouse_click(5, 5));
        s.tick();
        s.capture_frame();
    };

    #[allow(clippy::needless_borrows_for_generic_args)]
    let recording = Lab::record(config.clone(), ReplayTestModel::new(), &scenario);
    let result = Lab::replay(&recording, ReplayTestModel::new(), scenario);

    assert!(
        result.matched,
        "Replay must match recording. Divergence at frame {:?}: {:?}",
        result.first_divergence, result.divergence_detail
    );
    assert_eq!(result.frames_compared, 4);
    assert!(result.first_divergence.is_none());
}

/// Record with many frames, replay must match all.
#[test]
fn replay_50_frame_animation() {
    let recording = record_and_replay("animation_50", 6001, |s| {
        s.init();
        for i in 0..50 {
            if i % 5 == 0 {
                s.inject_event(key((b'a' + (i % 26) as u8) as char));
            }
            s.tick();
            s.capture_frame();
        }
    });

    assert_eq!(recording.frame_records.len(), 50);
}

// ============================================================================
// Edge cases
// ============================================================================

/// Empty scenario with no events.
#[test]
fn replay_empty_scenario() {
    let recording = record_and_replay("empty", 8001, |s| {
        s.init();
        s.tick();
        s.capture_frame();
    });

    assert_eq!(recording.frame_records.len(), 1);
}

/// Scenario with only init, no ticks.
#[test]
fn replay_init_only() {
    let recording = record_and_replay("init_only", 8002, |s| {
        s.init();
        s.capture_frame();
    });

    assert_eq!(recording.frame_records.len(), 1);
}

/// Rapid events without any ticks between them.
#[test]
fn replay_events_without_ticks() {
    let recording = record_and_replay("no_ticks", 8003, |s| {
        s.init();
        for c in "abcdefghij".chars() {
            s.inject_event(key(c));
        }
        // No ticks — model processes events during next tick
        s.tick();
        s.capture_frame();
    });

    assert_eq!(recording.frame_records.len(), 1);
}
