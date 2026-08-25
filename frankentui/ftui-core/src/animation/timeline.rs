#![forbid(unsafe_code)]

//! Timeline: multi-event animation scheduler.
//!
//! A [`Timeline`] sequences multiple [`Animation`] events at specified offsets,
//! with support for looping, pause/resume, and seek. The timeline itself
//! implements [`Animation`], producing a progress value (0.0–1.0) based on
//! elapsed time vs total duration.
//!
//! # Usage
//!
//! ```ignore
//! use std::time::Duration;
//! use ftui_core::animation::{Fade, Timeline};
//!
//! let timeline = Timeline::new()
//!     .add(Duration::ZERO, Fade::new(Duration::from_millis(500)))
//!     .add(Duration::from_millis(300), Fade::new(Duration::from_millis(400)))
//!     .duration(Duration::from_millis(700));
//!
//! // Events at [0..500ms] and [300..700ms] overlap.
//! ```
//!
//! # Invariants
//!
//! 1. Events are always sorted by offset (maintained on insertion).
//! 2. `value()` returns 0.0 when idle, and `current_time / duration` when
//!    playing (clamped to [0.0, 1.0]).
//! 3. `tick()` only advances animations in `Playing` state.
//! 4. Loop counter decrements only when current_time reaches duration.
//! 5. `seek()` clamps to [0, duration] and re-ticks all animations from
//!    their last state.
//!
//! # Failure Modes
//!
//! - Zero duration: clamped to 1ns to avoid division by zero.
//! - Seek past end: clamps to duration.
//! - Empty timeline: progress is always 1.0 (immediately complete).

use std::time::Duration;

use super::Animation;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How many times to loop the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopCount {
    /// Play once (no looping).
    Once,
    /// Repeat a fixed number of times (total plays = times + 1).
    Times(u32),
    /// Loop forever.
    Infinite,
}

/// Playback state of the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    /// Not yet started.
    Idle,
    /// Actively playing.
    Playing,
    /// Paused; can be resumed.
    Paused,
    /// Reached the end (all loops exhausted).
    Finished,
}

/// A single event in the timeline.
struct TimelineEvent {
    /// When this event starts relative to timeline start.
    offset: Duration,
    /// The animation for this event.
    animation: Box<dyn Animation>,
    /// Optional label for seeking by name.
    label: Option<String>,
}

impl std::fmt::Debug for TimelineEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimelineEvent")
            .field("offset", &self.offset)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// A timeline that schedules multiple animations at specific offsets.
///
/// Implements [`Animation`] where `value()` returns overall progress (0.0–1.0).
pub struct Timeline {
    events: Vec<TimelineEvent>,
    /// Total duration of the timeline. If not set explicitly, computed as
    /// max(event.offset) (animation durations are unknown without ticking).
    total_duration: Duration,
    /// Whether total_duration was explicitly set by the user.
    duration_explicit: bool,
    loop_count: LoopCount,
    /// Remaining loops (decremented during playback).
    loops_remaining: u32,
    state: PlaybackState,
    current_time: Duration,
}

impl std::fmt::Debug for Timeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Timeline")
            .field("event_count", &self.events.len())
            .field("total_duration", &self.total_duration)
            .field("loop_count", &self.loop_count)
            .field("state", &self.state)
            .field("current_time", &self.current_time)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl Timeline {
    /// Create an empty timeline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            total_duration: Duration::from_nanos(1),
            duration_explicit: false,
            loop_count: LoopCount::Once,
            loops_remaining: 0,
            state: PlaybackState::Idle,
            current_time: Duration::ZERO,
        }
    }

    /// Add an animation event at an absolute offset (builder pattern).
    #[must_use]
    pub fn add(mut self, offset: Duration, animation: impl Animation + 'static) -> Self {
        self.push_event(offset, Box::new(animation), None);
        self
    }

    /// Add a labeled animation event at an absolute offset (builder pattern).
    #[must_use]
    pub fn add_labeled(
        mut self,
        label: &str,
        offset: Duration,
        animation: impl Animation + 'static,
    ) -> Self {
        self.push_event(offset, Box::new(animation), Some(label.to_string()));
        self
    }

    /// Add an event relative to the last event's offset (builder pattern).
    ///
    /// If no events exist, the offset is 0.
    #[must_use]
    pub fn then(self, animation: impl Animation + 'static) -> Self {
        let offset = self.events.last().map_or(Duration::ZERO, |e| e.offset);
        self.add(offset, animation)
    }

    /// Set the total duration explicitly (builder pattern).
    ///
    /// If not called, duration is inferred as `max(event.offset)`.
    /// A zero duration is clamped to 1ns.
    #[must_use]
    pub fn set_duration(mut self, d: Duration) -> Self {
        self.total_duration = if d.is_zero() {
            Duration::from_nanos(1)
        } else {
            d
        };
        self.duration_explicit = true;
        self
    }

    /// Set the loop count (builder pattern).
    #[must_use]
    pub fn set_loop_count(mut self, count: LoopCount) -> Self {
        self.loop_count = count;
        self.loops_remaining = match count {
            LoopCount::Once => 0,
            LoopCount::Times(n) => n,
            LoopCount::Infinite => u32::MAX,
        };
        self
    }

    /// Internal: insert event maintaining sort order by offset.
    fn push_event(
        &mut self,
        offset: Duration,
        animation: Box<dyn Animation>,
        label: Option<String>,
    ) {
        let event = TimelineEvent {
            offset,
            animation,
            label,
        };
        // Insert sorted by offset (stable — preserves insertion order for same offset).
        let pos = self.events.partition_point(|e| e.offset <= offset);
        self.events.insert(pos, event);

        // Auto-compute duration if not explicitly set.
        if !self.duration_explicit {
            self.total_duration = self.events.last().map_or(Duration::from_nanos(1), |e| {
                if e.offset.is_zero() {
                    Duration::from_nanos(1)
                } else {
                    e.offset
                }
            });
        }
    }
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Playback control
// ---------------------------------------------------------------------------

impl Timeline {
    /// Start or restart playback from the beginning.
    pub fn play(&mut self) {
        self.current_time = Duration::ZERO;
        self.loops_remaining = match self.loop_count {
            LoopCount::Once => 0,
            LoopCount::Times(n) => n,
            LoopCount::Infinite => u32::MAX,
        };
        for event in &mut self.events {
            event.animation.reset();
        }
        self.state = PlaybackState::Playing;
    }

    /// Pause playback. No-op if not playing.
    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
        }
    }

    /// Resume from pause. No-op if not paused.
    pub fn resume(&mut self) {
        if self.state == PlaybackState::Paused {
            self.state = PlaybackState::Playing;
        }
    }

    /// Stop playback and reset to idle.
    pub fn stop(&mut self) {
        self.state = PlaybackState::Idle;
        self.current_time = Duration::ZERO;
        for event in &mut self.events {
            event.animation.reset();
        }
    }

    /// Seek to an absolute time position.
    ///
    /// Clamps to [0, total_duration]. Resets all animations and re-ticks
    /// them up to the seek point so their state is consistent.
    pub fn seek(&mut self, time: Duration) {
        let clamped = if time > self.total_duration {
            self.total_duration
        } else {
            time
        };

        // Reset all animations, then re-tick them up to `clamped`.
        for event in &mut self.events {
            event.animation.reset();
            if clamped > event.offset {
                let dt = clamped.saturating_sub(event.offset);
                event.animation.tick(dt);
            }
        }
        self.current_time = clamped;

        // If we were idle/finished, transition to paused at the seek point.
        if self.state == PlaybackState::Idle || self.state == PlaybackState::Finished {
            self.state = PlaybackState::Paused;
        }
    }

    /// Seek to a labeled event's offset.
    ///
    /// Returns `true` if the label was found, `false` otherwise (no-op).
    pub fn seek_label(&mut self, label: &str) -> bool {
        let offset = self
            .events
            .iter()
            .find(|e| e.label.as_deref() == Some(label))
            .map(|e| e.offset);
        if let Some(offset) = offset {
            self.seek(offset);
            true
        } else {
            false
        }
    }

    /// Current progress as a value in [0.0, 1.0].
    #[inline]
    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.events.is_empty() {
            return 1.0;
        }
        if self.total_duration.is_zero() {
            return 1.0;
        }
        let t = self.current_time.as_secs_f64() / self.total_duration.as_secs_f64();
        (t as f32).clamp(0.0, 1.0)
    }

    /// Current playback state.
    #[inline]
    #[must_use]
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Current time position.
    #[inline]
    #[must_use]
    pub fn current_time(&self) -> Duration {
        self.current_time
    }

    /// Total duration.
    #[inline]
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.total_duration
    }

    /// Number of events in the timeline.
    #[inline]
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Get the animation value for a specific labeled event.
    ///
    /// Returns `None` if the label doesn't exist.
    #[must_use]
    pub fn event_value(&self, label: &str) -> Option<f32> {
        self.events
            .iter()
            .find(|e| e.label.as_deref() == Some(label))
            .map(|e| e.animation.value())
    }

    /// Get the animation value for an event by index.
    ///
    /// Returns `None` if index is out of bounds.
    #[must_use]
    pub fn event_value_at(&self, index: usize) -> Option<f32> {
        self.events.get(index).map(|e| e.animation.value())
    }
}

// ---------------------------------------------------------------------------
// Animation trait implementation
// ---------------------------------------------------------------------------

impl Animation for Timeline {
    fn tick(&mut self, dt: Duration) {
        if self.state != PlaybackState::Playing {
            return;
        }

        let new_time = self.current_time.saturating_add(dt);

        // Tick each event that overlaps with [current_time, new_time].
        for event in &mut self.events {
            if new_time > event.offset && !event.animation.is_complete() {
                // How much time has elapsed for this event.
                let event_start = event.offset;
                if self.current_time >= event_start {
                    // Already past offset — just forward dt.
                    event.animation.tick(dt);
                } else {
                    // Event starts within this tick — forward only the portion after offset.
                    let partial = new_time.saturating_sub(event_start);
                    event.animation.tick(partial);
                }
            }
        }

        self.current_time = new_time;

        // Check if we've reached the end of the timeline.
        if self.current_time >= self.total_duration {
            match self.loop_count {
                LoopCount::Once => {
                    self.current_time = self.total_duration;
                    self.state = PlaybackState::Finished;
                }
                LoopCount::Times(_) | LoopCount::Infinite => {
                    if self.loops_remaining > 0 {
                        if self.loop_count != LoopCount::Infinite {
                            self.loops_remaining -= 1;
                        }
                        // Calculate overshoot to carry into next loop.
                        let overshoot = self.current_time.saturating_sub(self.total_duration);
                        self.current_time = Duration::ZERO;
                        for event in &mut self.events {
                            event.animation.reset();
                        }
                        // Apply overshoot to next loop.
                        if !overshoot.is_zero() {
                            self.tick(overshoot);
                        }
                    } else {
                        self.current_time = self.total_duration;
                        self.state = PlaybackState::Finished;
                    }
                }
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.state == PlaybackState::Finished
    }

    fn value(&self) -> f32 {
        self.progress()
    }

    fn reset(&mut self) {
        self.current_time = Duration::ZERO;
        self.loops_remaining = match self.loop_count {
            LoopCount::Once => 0,
            LoopCount::Times(n) => n,
            LoopCount::Infinite => u32::MAX,
        };
        self.state = PlaybackState::Idle;
        for event in &mut self.events {
            event.animation.reset();
        }
    }

    fn overshoot(&self) -> Duration {
        if self.state == PlaybackState::Finished {
            self.current_time.saturating_sub(self.total_duration)
        } else {
            Duration::ZERO
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Fade;

    const MS_100: Duration = Duration::from_millis(100);
    const MS_200: Duration = Duration::from_millis(200);
    const MS_250: Duration = Duration::from_millis(250);
    const MS_300: Duration = Duration::from_millis(300);
    const MS_500: Duration = Duration::from_millis(500);
    const SEC_1: Duration = Duration::from_secs(1);

    #[test]
    fn empty_timeline_is_immediately_complete() {
        let tl = Timeline::new();
        assert_eq!(tl.progress(), 1.0);
        assert_eq!(tl.event_count(), 0);
    }

    #[test]
    fn sequential_events() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_200))
            .add(MS_200, Fade::new(MS_200))
            .add(Duration::from_millis(400), Fade::new(MS_200))
            .set_duration(Duration::from_millis(600));

        tl.play();

        // At 100ms: first event at 50%, others not started
        tl.tick(MS_100);
        assert!((tl.event_value_at(0).unwrap() - 0.5).abs() < 0.01);
        assert!((tl.event_value_at(1).unwrap() - 0.0).abs() < 0.01);

        // At 300ms: first complete, second at 50%
        tl.tick(MS_200);
        assert!(tl.event_value_at(0).unwrap() > 0.99);
        assert!((tl.event_value_at(1).unwrap() - 0.5).abs() < 0.01);

        // At 600ms: all complete
        tl.tick(MS_300);
        assert!(tl.is_complete());
        assert!((tl.progress() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn overlapping_events() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .add(MS_200, Fade::new(MS_500))
            .set_duration(Duration::from_millis(700));

        tl.play();

        // At 300ms: first at 60%, second at 20%
        tl.tick(MS_300);
        assert!((tl.event_value_at(0).unwrap() - 0.6).abs() < 0.02);
        assert!((tl.event_value_at(1).unwrap() - 0.2).abs() < 0.02);
    }

    #[test]
    fn labeled_events_and_seek() {
        let mut tl = Timeline::new()
            .add_labeled("intro", Duration::ZERO, Fade::new(MS_500))
            .add_labeled("main", MS_500, Fade::new(MS_500))
            .set_duration(SEC_1);

        tl.play();

        // Seek to "main"
        assert!(tl.seek_label("main"));
        // First animation should have been ticked for full 500ms
        assert!(tl.event_value("intro").unwrap() > 0.99);
        // Main just started
        assert!((tl.event_value("main").unwrap() - 0.0).abs() < f32::EPSILON);

        // Unknown label returns false
        assert!(!tl.seek_label("nonexistent"));
    }

    #[test]
    fn loop_finite() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100)
            .set_loop_count(LoopCount::Times(2));

        tl.play();

        // First play-through
        tl.tick(MS_100);
        assert!(!tl.is_complete());
        assert_eq!(tl.state(), PlaybackState::Playing);

        // Second play-through (first loop)
        tl.tick(MS_100);
        assert!(!tl.is_complete());

        // Third play-through (second loop) — should finish
        tl.tick(MS_100);
        assert!(tl.is_complete());
    }

    #[test]
    fn loop_infinite_never_finishes() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100)
            .set_loop_count(LoopCount::Infinite);

        tl.play();

        // Run through many cycles
        for _ in 0..100 {
            tl.tick(MS_100);
        }
        assert!(!tl.is_complete());
        assert_eq!(tl.state(), PlaybackState::Playing);
    }

    #[test]
    fn pause_resume() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(SEC_1))
            .set_duration(SEC_1);

        tl.play();
        tl.tick(MS_250);

        tl.pause();
        assert_eq!(tl.state(), PlaybackState::Paused);
        let time_at_pause = tl.current_time();

        // Tick while paused — should not advance
        tl.tick(MS_500);
        assert_eq!(tl.current_time(), time_at_pause);

        tl.resume();
        assert_eq!(tl.state(), PlaybackState::Playing);

        // Now ticks advance again
        tl.tick(MS_250);
        assert!(tl.current_time() > time_at_pause);
    }

    #[test]
    fn seek_clamps_to_duration() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.play();
        tl.seek(SEC_1); // Past end
        assert_eq!(tl.current_time(), MS_500);
    }

    #[test]
    fn seek_resets_and_reticks_animations() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(SEC_1))
            .set_duration(SEC_1);

        tl.play();
        tl.tick(MS_500);
        // Event at ~50%
        assert!((tl.event_value_at(0).unwrap() - 0.5).abs() < 0.02);

        // Seek back to 250ms
        tl.seek(MS_250);
        assert!((tl.event_value_at(0).unwrap() - 0.25).abs() < 0.02);
    }

    #[test]
    fn stop_resets_everything() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(SEC_1))
            .set_duration(SEC_1);

        tl.play();
        tl.tick(MS_500);
        tl.stop();

        assert_eq!(tl.state(), PlaybackState::Idle);
        assert_eq!(tl.current_time(), Duration::ZERO);
        assert!((tl.event_value_at(0).unwrap() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn play_restarts_from_beginning() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(SEC_1))
            .set_duration(SEC_1);

        tl.play();
        tl.tick(SEC_1);
        assert!(tl.is_complete());

        tl.play();
        assert_eq!(tl.state(), PlaybackState::Playing);
        assert_eq!(tl.current_time(), Duration::ZERO);
        assert!((tl.event_value_at(0).unwrap() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn then_chains_at_same_offset() {
        let tl = Timeline::new()
            .add(MS_100, Fade::new(MS_100))
            .then(Fade::new(MS_100)); // Should be at offset 100ms too

        assert_eq!(tl.event_count(), 2);
    }

    #[test]
    fn progress_tracks_time() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(SEC_1))
            .set_duration(SEC_1);

        tl.play();
        assert!((tl.progress() - 0.0).abs() < f32::EPSILON);

        tl.tick(MS_250);
        assert!((tl.progress() - 0.25).abs() < 0.02);

        tl.tick(MS_250);
        assert!((tl.progress() - 0.5).abs() < 0.02);
    }

    #[test]
    fn animation_trait_value_matches_progress() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(SEC_1))
            .set_duration(SEC_1);

        tl.play();
        tl.tick(MS_500);

        assert!((tl.value() - tl.progress()).abs() < f32::EPSILON);
    }

    #[test]
    fn animation_trait_reset() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(SEC_1))
            .set_duration(SEC_1);

        tl.play();
        tl.tick(SEC_1);
        assert!(tl.is_complete());

        tl.reset();
        assert_eq!(tl.state(), PlaybackState::Idle);
        assert!(!tl.is_complete());
    }

    #[test]
    fn debug_format() {
        let tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        let dbg = format!("{:?}", tl);
        assert!(dbg.contains("Timeline"));
        assert!(dbg.contains("event_count"));
    }

    #[test]
    fn loop_once_plays_exactly_once() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100)
            .set_loop_count(LoopCount::Once);

        tl.play();
        tl.tick(MS_100);
        assert!(tl.is_complete());
    }

    #[test]
    fn event_value_by_label_missing_returns_none() {
        let tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        assert!(tl.event_value("nope").is_none());
    }

    #[test]
    fn event_value_at_out_of_bounds() {
        let tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        assert!(tl.event_value_at(5).is_none());
    }

    #[test]
    fn idle_timeline_value_is_zero() {
        let tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        // Not yet played: value should be 0.0
        assert!((tl.value() - 0.0).abs() < f32::EPSILON);
        assert_eq!(tl.state(), PlaybackState::Idle);
    }

    #[test]
    fn overshoot_is_zero_while_playing() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.play();
        tl.tick(MS_250);

        assert_eq!(tl.overshoot(), Duration::ZERO);
    }

    #[test]
    fn seek_to_zero_resets_animations() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.play();
        tl.tick(MS_250);
        assert!(tl.event_value_at(0).unwrap() > 0.0);

        tl.seek(Duration::ZERO);
        assert_eq!(tl.current_time(), Duration::ZERO);
    }

    // ── Edge-case tests (bd-nq7yt) ──────────────────────────────────

    #[test]
    fn default_trait() {
        let tl = Timeline::default();
        assert_eq!(tl.event_count(), 0);
        assert_eq!(tl.state(), PlaybackState::Idle);
        assert_eq!(tl.progress(), 1.0); // empty timeline is immediately complete
    }

    #[test]
    fn zero_duration_clamped_to_1ns() {
        let tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(Duration::ZERO);
        assert_eq!(tl.duration(), Duration::from_nanos(1));
    }

    #[test]
    fn then_on_empty_timeline_uses_zero_offset() {
        let tl = Timeline::new().then(Fade::new(MS_100));
        assert_eq!(tl.event_count(), 1);
        // Auto-computed duration from a single event at offset 0 → 1ns.
        assert_eq!(tl.duration(), Duration::from_nanos(1));
    }

    #[test]
    fn pause_when_not_playing_is_noop() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        // Pause from Idle → still Idle.
        tl.pause();
        assert_eq!(tl.state(), PlaybackState::Idle);

        // Pause from Finished → still Finished.
        tl.play();
        tl.tick(MS_100);
        assert_eq!(tl.state(), PlaybackState::Finished);
        tl.pause();
        assert_eq!(tl.state(), PlaybackState::Finished);
    }

    #[test]
    fn resume_when_not_paused_is_noop() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        // Resume from Idle → still Idle.
        tl.resume();
        assert_eq!(tl.state(), PlaybackState::Idle);

        // Resume from Playing → still Playing.
        tl.play();
        tl.resume();
        assert_eq!(tl.state(), PlaybackState::Playing);
    }

    #[test]
    fn seek_from_idle_transitions_to_paused() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        assert_eq!(tl.state(), PlaybackState::Idle);
        tl.seek(MS_250);
        assert_eq!(tl.state(), PlaybackState::Paused);
        assert_eq!(tl.current_time(), MS_250);
    }

    #[test]
    fn seek_from_finished_transitions_to_paused() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.play();
        tl.tick(MS_500);
        assert_eq!(tl.state(), PlaybackState::Finished);

        tl.seek(MS_250);
        assert_eq!(tl.state(), PlaybackState::Paused);
        assert_eq!(tl.current_time(), MS_250);
    }

    #[test]
    fn seek_from_playing_stays_playing() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.play();
        tl.tick(MS_100);
        assert_eq!(tl.state(), PlaybackState::Playing);

        tl.seek(MS_300);
        // Seek doesn't change Playing state.
        assert_eq!(tl.state(), PlaybackState::Playing);
    }

    #[test]
    fn overshoot_when_finished() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        tl.play();
        tl.tick(MS_100);
        assert!(tl.is_complete());
        // LoopCount::Once clamps current_time to duration, so overshoot is 0.
        assert_eq!(tl.overshoot(), Duration::ZERO);
    }

    #[test]
    fn tick_when_idle_does_not_advance() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.tick(MS_250);
        assert_eq!(tl.current_time(), Duration::ZERO);
        assert_eq!(tl.state(), PlaybackState::Idle);
    }

    #[test]
    fn tick_when_paused_does_not_advance() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(SEC_1))
            .set_duration(SEC_1);

        tl.play();
        tl.tick(MS_250);
        tl.pause();
        let paused_time = tl.current_time();

        tl.tick(MS_500);
        assert_eq!(tl.current_time(), paused_time);
    }

    #[test]
    fn tick_when_finished_does_not_advance() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        tl.play();
        tl.tick(MS_100);
        assert!(tl.is_complete());

        let time_at_finish = tl.current_time();
        tl.tick(MS_500);
        assert_eq!(tl.current_time(), time_at_finish);
    }

    #[test]
    fn multiple_events_at_same_offset() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_200))
            .add(Duration::ZERO, Fade::new(MS_200))
            .add(Duration::ZERO, Fade::new(MS_200))
            .set_duration(MS_200);

        assert_eq!(tl.event_count(), 3);
        tl.play();
        tl.tick(MS_100);

        // All three should be at ~50%.
        for i in 0..3 {
            assert!(
                (tl.event_value_at(i).unwrap() - 0.5).abs() < 0.02,
                "event {i} should be at ~50%"
            );
        }
    }

    #[test]
    fn auto_computed_duration_uses_max_offset() {
        let tl = Timeline::new()
            .add(MS_100, Fade::new(MS_100))
            .add(MS_500, Fade::new(MS_100))
            .add(MS_300, Fade::new(MS_100));

        // Events sorted by offset: 100, 300, 500.
        // Duration should be max offset = 500ms.
        assert_eq!(tl.duration(), MS_500);
    }

    #[test]
    fn explicit_duration_overrides_auto() {
        let tl = Timeline::new()
            .add(MS_100, Fade::new(MS_100))
            .add(MS_500, Fade::new(MS_100))
            .set_duration(SEC_1);

        assert_eq!(tl.duration(), SEC_1);
    }

    #[test]
    fn seek_label_on_empty_timeline() {
        let mut tl = Timeline::new();
        assert!(!tl.seek_label("foo"));
    }

    #[test]
    fn event_value_at_on_empty_timeline() {
        let tl = Timeline::new();
        assert!(tl.event_value_at(0).is_none());
    }

    #[test]
    fn loop_times_zero_plays_once() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100)
            .set_loop_count(LoopCount::Times(0));

        tl.play();
        tl.tick(MS_100);
        assert!(tl.is_complete());
    }

    #[test]
    fn loop_count_eq() {
        assert_eq!(LoopCount::Once, LoopCount::Once);
        assert_eq!(LoopCount::Times(5), LoopCount::Times(5));
        assert_ne!(LoopCount::Times(5), LoopCount::Times(3));
        assert_eq!(LoopCount::Infinite, LoopCount::Infinite);
        assert_ne!(LoopCount::Once, LoopCount::Infinite);
    }

    #[test]
    fn playback_state_eq() {
        assert_eq!(PlaybackState::Idle, PlaybackState::Idle);
        assert_eq!(PlaybackState::Playing, PlaybackState::Playing);
        assert_eq!(PlaybackState::Paused, PlaybackState::Paused);
        assert_eq!(PlaybackState::Finished, PlaybackState::Finished);
        assert_ne!(PlaybackState::Idle, PlaybackState::Playing);
    }

    #[test]
    fn loop_count_clone() {
        let lc = LoopCount::Times(3);
        let lc2 = lc;
        assert_eq!(lc, lc2);
    }

    #[test]
    fn playback_state_clone() {
        let ps = PlaybackState::Paused;
        let ps2 = ps;
        assert_eq!(ps, ps2);
    }

    #[test]
    fn play_after_stop_resets() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.play();
        tl.tick(MS_250);
        tl.stop();
        assert_eq!(tl.state(), PlaybackState::Idle);

        tl.play();
        assert_eq!(tl.state(), PlaybackState::Playing);
        assert_eq!(tl.current_time(), Duration::ZERO);
    }

    #[test]
    fn seek_past_end_then_resume_and_tick_finishes() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        tl.play();
        tl.seek(MS_100); // Seek to end, stays Playing.
        tl.resume(); // No-op if already playing.

        // Tick should detect we're at duration and finish.
        tl.tick(Duration::from_nanos(1));
        assert!(tl.is_complete());
    }

    #[test]
    fn progress_clamps_to_zero_one() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        // Before play: progress 0.0.
        assert!(tl.progress() >= 0.0);
        assert!(tl.progress() <= 1.0);

        tl.play();
        tl.tick(MS_100);
        assert!(tl.progress() >= 0.0);
        assert!(tl.progress() <= 1.0);
    }

    #[test]
    fn animation_trait_is_complete_false_while_playing() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.play();
        tl.tick(MS_250);
        assert!(!tl.is_complete());
    }

    #[test]
    fn reset_from_finished() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        tl.play();
        tl.tick(MS_100);
        assert!(tl.is_complete());

        tl.reset();
        assert_eq!(tl.state(), PlaybackState::Idle);
        assert_eq!(tl.current_time(), Duration::ZERO);
        assert!(!tl.is_complete());
    }

    #[test]
    fn reset_from_paused() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_500))
            .set_duration(MS_500);

        tl.play();
        tl.tick(MS_250);
        tl.pause();
        assert_eq!(tl.state(), PlaybackState::Paused);

        tl.reset();
        assert_eq!(tl.state(), PlaybackState::Idle);
        assert_eq!(tl.current_time(), Duration::ZERO);
    }

    #[test]
    fn labeled_event_value() {
        let mut tl = Timeline::new()
            .add_labeled("fade", Duration::ZERO, Fade::new(MS_200))
            .set_duration(MS_200);

        tl.play();
        tl.tick(MS_100);

        let v = tl.event_value("fade").unwrap();
        assert!((v - 0.5).abs() < 0.02);
    }

    #[test]
    fn events_sorted_by_offset_on_insert() {
        let tl = Timeline::new()
            .add(MS_500, Fade::new(MS_100))
            .add(MS_100, Fade::new(MS_100))
            .add(MS_300, Fade::new(MS_100));

        // Event at index 0 should be the one at smallest offset.
        // After ticking past 100ms, event_value_at(0) should advance.
        let mut tl = tl.set_duration(MS_500);
        tl.play();
        tl.tick(Duration::from_millis(150));

        // Event 0 (offset 100ms): should be at 50ms into its 100ms fade = 50%.
        let v = tl.event_value_at(0).unwrap();
        assert!((v - 0.5).abs() < 0.02);

        // Event 1 (offset 300ms): not started yet.
        let v = tl.event_value_at(1).unwrap();
        assert!(v < 0.01);
    }

    #[test]
    fn debug_format_includes_fields() {
        let tl = Timeline::new()
            .add_labeled("intro", Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100);

        let dbg = format!("{:?}", tl);
        assert!(dbg.contains("event_count"));
        assert!(dbg.contains("total_duration"));
        assert!(dbg.contains("state"));
    }

    #[test]
    fn loop_finite_with_overshoot_tick() {
        let mut tl = Timeline::new()
            .add(Duration::ZERO, Fade::new(MS_100))
            .set_duration(MS_100)
            .set_loop_count(LoopCount::Times(1));

        tl.play();
        // Tick 150ms in one go — overshoots first play by 50ms.
        tl.tick(Duration::from_millis(150));
        // Should be in second play (first loop), not finished.
        assert!(!tl.is_complete());
        // Current time should be 50ms into the second play.
        assert!(tl.current_time() <= MS_100);
    }
}
