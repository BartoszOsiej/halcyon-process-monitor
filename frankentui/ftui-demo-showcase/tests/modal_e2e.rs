#![forbid(unsafe_code)]

//! End-to-end snapshot tests for the Modal Container Widget (bd-39vx.1).
//!
//! These tests verify the visual output of the Modal widget at various
//! configurations and screen sizes.
//!
//! # Snapshot Tests
//!
//! - `modal_center_80x24`: Modal centered at standard 80x24 terminal size
//! - `modal_offset_80x24`: Modal with offset positioning
//! - `modal_constrained_120x40`: Modal with size constraints at larger size
//! - `modal_backdrop_opacity`: Modal with custom backdrop opacity
//!
//! # Invariants (Alien Artifact)
//!
//! 1. **Positioning**: Modal content is always clamped within the available area
//! 2. **Size constraints**: min/max width/height never exceed available space
//! 3. **Hit regions**: Backdrop and content hit regions are correctly registered
//! 4. **Backdrop rendering**: Backdrop covers entire area; content renders on top
//!
//! Run: `cargo test -p ftui-demo-showcase --test modal_e2e`
//! Update snapshots: `BLESS=1 cargo test -p ftui-demo-showcase --test modal_e2e`

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_harness::assert_snapshot;
use ftui_harness::golden::compute_buffer_checksum;
use ftui_render::cell::PackedRgba;
use ftui_render::frame::{Frame, HitData, HitId, HitRegion};
use ftui_render::grapheme_pool::GraphemePool;
use ftui_widgets::block::Block;
use ftui_widgets::borders::{BorderType, Borders};
use ftui_widgets::focus::FocusNode;
use ftui_widgets::modal::{
    BackdropConfig, Dialog, DialogResult, DialogState, FocusAwareModalStack, Modal, ModalPosition,
    ModalResultData, ModalSizeConstraints, ModalStack, StackModal, WidgetModalEntry,
};
use ftui_widgets::paragraph::Paragraph;
use ftui_widgets::{StatefulWidget, Widget};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Helper: Content widget for modal tests
// ---------------------------------------------------------------------------

/// A simple bordered dialog content for testing.
struct DialogContent<'a> {
    title: &'a str,
    message: &'a str,
}

impl<'a> DialogContent<'a> {
    fn new(title: &'a str, message: &'a str) -> Self {
        Self { title, message }
    }
}

impl Widget for DialogContent<'_> {
    fn render(&self, area: Rect, frame: &mut Frame) {
        // Draw block with rounded borders
        let block = Block::new()
            .title(self.title)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded);

        block.render(area, frame);

        // Render message in inner area
        let inner = block.inner(area);
        if !inner.is_empty() {
            let paragraph = Paragraph::new(self.message);
            paragraph.render(inner, frame);
        }
    }
}

fn sample_content() -> DialogContent<'static> {
    DialogContent::new(
        " Modal Dialog ",
        "This is sample modal content.\nPress Esc to close.",
    )
}

fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: Modifiers::NONE,
        kind: KeyEventKind::Press,
    })
}

fn char_press(ch: char) -> Event {
    press(KeyCode::Char(ch))
}

fn log_jsonl(step: &str, data: &[(&str, &str)]) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = COUNTER.fetch_add(1, Ordering::Relaxed);
    let fields: Vec<String> = std::iter::once(format!("\"ts\":\"T{ts:06}\""))
        .chain(std::iter::once(format!("\"step\":\"{step}\"")))
        .chain(data.iter().map(|(k, v)| format!("\"{k}\":\"{v}\"")))
        .collect();
    eprintln!("{{{}}}", fields.join(","));
}

fn log_snapshot(name: &str, frame: &Frame<'_>) {
    let checksum = compute_buffer_checksum(&frame.buffer);
    log_jsonl(
        "snapshot",
        &[("name", name), ("checksum", checksum.as_str())],
    );
}

fn render_dialog(
    dialog: &Dialog,
    state: &mut DialogState,
    width: u16,
    height: u16,
) -> Frame<'static> {
    // For tests, leak the pool to satisfy the frame's lifetime.
    let pool = Box::leak(Box::new(GraphemePool::new()));
    let mut frame = Frame::new(width, height, pool);
    let area = Rect::new(0, 0, width, height);
    dialog.render(area, &mut frame, state);
    frame
}

// ===========================================================================
// Snapshot: modal_center_80x24
// ===========================================================================

#[test]
fn modal_center_80x24() {
    let modal = Modal::new(sample_content()).size(
        ModalSizeConstraints::new()
            .min_width(30)
            .max_width(40)
            .min_height(8)
            .max_height(10),
    );

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 24, &mut pool);
    let area = Rect::new(0, 0, 80, 24);

    // Render backdrop area with some content to see the overlay effect
    fill_background(&mut frame, area);

    modal.render(area, &mut frame);
    log_snapshot("modal_center_80x24", &frame);
    assert_snapshot!("modal_center_80x24", &frame.buffer);
}

// ===========================================================================
// Snapshot: modal_offset_80x24
// ===========================================================================

#[test]
fn modal_offset_80x24() {
    let modal = Modal::new(sample_content())
        .size(
            ModalSizeConstraints::new()
                .min_width(30)
                .max_width(40)
                .min_height(8)
                .max_height(10),
        )
        .position(ModalPosition::CenterOffset { x: -10, y: -3 });

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 24, &mut pool);
    let area = Rect::new(0, 0, 80, 24);

    fill_background(&mut frame, area);
    modal.render(area, &mut frame);
    log_snapshot("modal_offset_80x24", &frame);
    assert_snapshot!("modal_offset_80x24", &frame.buffer);
}

// ===========================================================================
// Snapshot: modal_constrained_120x40
// ===========================================================================

#[test]
fn modal_constrained_120x40() {
    let modal = Modal::new(sample_content())
        .size(
            ModalSizeConstraints::new()
                .min_width(50)
                .max_width(80)
                .min_height(15)
                .max_height(25),
        )
        .position(ModalPosition::TopCenter { margin: 3 });

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(120, 40, &mut pool);
    let area = Rect::new(0, 0, 120, 40);

    fill_background(&mut frame, area);
    modal.render(area, &mut frame);
    log_snapshot("modal_constrained_120x40", &frame);
    assert_snapshot!("modal_constrained_120x40", &frame.buffer);
}

// ===========================================================================
// Snapshot: modal_backdrop_opacity
// ===========================================================================

#[test]
fn modal_backdrop_opacity() {
    let modal = Modal::new(sample_content())
        .size(
            ModalSizeConstraints::new()
                .min_width(35)
                .max_width(45)
                .min_height(10)
                .max_height(12),
        )
        .backdrop(BackdropConfig::new(PackedRgba::rgb(0, 0, 128), 0.8));

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 24, &mut pool);
    let area = Rect::new(0, 0, 80, 24);

    fill_background(&mut frame, area);
    modal.render(area, &mut frame);
    log_snapshot("modal_backdrop_opacity", &frame);
    assert_snapshot!("modal_backdrop_opacity", &frame.buffer);
}

// ===========================================================================
// Additional tests: edge cases and invariants
// ===========================================================================

#[test]
fn modal_tiny_40x10() {
    let modal = Modal::new(sample_content()).size(
        ModalSizeConstraints::new()
            .min_width(20)
            .max_width(35)
            .min_height(6)
            .max_height(8),
    );

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(40, 10, &mut pool);
    let area = Rect::new(0, 0, 40, 10);

    fill_background(&mut frame, area);
    modal.render(area, &mut frame);
    log_snapshot("modal_tiny_40x10", &frame);
    assert_snapshot!("modal_tiny_40x10", &frame.buffer);
}

// ===========================================================================
// Dialog E2E: Alert / Confirm / Prompt / Custom
// ===========================================================================

#[test]
fn dialog_alert_enter_confirms() {
    log_jsonl("env", &[("test", "dialog_alert_enter_confirms")]);
    let dialog = Dialog::alert("Alert", "Operation complete.");
    let mut state = DialogState::new();

    let result = dialog.handle_event(&press(KeyCode::Enter), &mut state, None);
    assert_eq!(result, Some(DialogResult::Ok));
    assert!(!state.is_open(), "Dialog should close after Enter");

    let mut state = DialogState::new();
    let frame = render_dialog(&dialog, &mut state, 80, 24);
    log_snapshot("dialog_alert_80x24", &frame);
    assert_snapshot!("dialog_alert_80x24", &frame.buffer);
}

#[test]
fn dialog_confirm_cancel_via_right_enter() {
    log_jsonl("env", &[("test", "dialog_confirm_cancel_via_right_enter")]);
    let dialog = Dialog::confirm("Confirm", "Are you sure?");
    let mut state = DialogState::new();

    state.input_focused = false;
    dialog.handle_event(&press(KeyCode::Right), &mut state, None);
    dialog.handle_event(&press(KeyCode::Right), &mut state, None);
    let result = dialog.handle_event(&press(KeyCode::Enter), &mut state, None);
    assert_eq!(result, Some(DialogResult::Cancel));
    assert!(!state.is_open());

    let mut state = DialogState::new();
    let frame = render_dialog(&dialog, &mut state, 80, 24);
    log_snapshot("dialog_confirm_80x24", &frame);
    assert_snapshot!("dialog_confirm_80x24", &frame.buffer);
}

#[test]
fn dialog_prompt_input_returns_value() {
    log_jsonl("env", &[("test", "dialog_prompt_input_returns_value")]);
    let dialog = Dialog::prompt("Prompt", "Enter name:");
    let mut state = DialogState::new();

    dialog.handle_event(&char_press('A'), &mut state, None);
    dialog.handle_event(&char_press('d'), &mut state, None);
    dialog.handle_event(&char_press('a'), &mut state, None);
    let result = dialog.handle_event(&press(KeyCode::Enter), &mut state, None);

    assert_eq!(result, Some(DialogResult::Input("Ada".into())));
    assert!(!state.is_open());

    let mut state = DialogState::new();
    let frame = render_dialog(&dialog, &mut state, 80, 24);
    log_snapshot("dialog_prompt_80x24", &frame);
    assert_snapshot!("dialog_prompt_80x24", &frame.buffer);
}

#[test]
fn dialog_escape_dismisses() {
    log_jsonl("env", &[("test", "dialog_escape_dismisses")]);
    let dialog = Dialog::alert("Alert", "Dismiss me.");
    let mut state = DialogState::new();

    let result = dialog.handle_event(&press(KeyCode::Escape), &mut state, None);
    assert_eq!(result, Some(DialogResult::Dismissed));
    assert!(!state.is_open());
}

// ===========================================================================
// Modal Stack E2E: LIFO Close Order + Focus Trap
// ===========================================================================

#[test]
fn modal_stack_lifo_escape_closes_top() {
    log_jsonl("env", &[("test", "modal_stack_lifo_escape_closes_top")]);
    let mut stack = ModalStack::new();
    let id1 = stack.push(Box::new(WidgetModalEntry::new(sample_content())));
    let id2 = stack.push(Box::new(WidgetModalEntry::new(sample_content())));

    let result = stack.handle_event(&press(KeyCode::Escape), None);
    assert!(result.is_some());
    assert_eq!(result.unwrap().id, id2);
    assert!(stack.contains(id1));
    assert!(!stack.contains(id2));
}

#[test]
fn modal_stack_render_z_order_80x24() {
    log_jsonl("env", &[("test", "modal_stack_render_z_order_80x24")]);
    let mut stack = ModalStack::new();

    let bottom = WidgetModalEntry::new(DialogContent::new(" Bottom ", "Lower layer"))
        .size(
            ModalSizeConstraints::new()
                .min_width(34)
                .max_width(42)
                .min_height(10)
                .max_height(12),
        )
        .backdrop(BackdropConfig::new(PackedRgba::rgb(64, 0, 0), 0.5));

    let top = WidgetModalEntry::new(DialogContent::new(" Top ", "Upper layer"))
        .size(
            ModalSizeConstraints::new()
                .min_width(26)
                .max_width(32)
                .min_height(8)
                .max_height(10),
        )
        .backdrop(BackdropConfig::new(PackedRgba::rgb(0, 0, 96), 0.7));

    stack.push(Box::new(bottom));
    stack.push(Box::new(top));

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 24, &mut pool);
    let area = Rect::new(0, 0, 80, 24);
    fill_background(&mut frame, area);
    stack.render(&mut frame, area);

    log_snapshot("modal_stack_z_order_80x24", &frame);
    assert_snapshot!("modal_stack_z_order_80x24", &frame.buffer);
}

#[derive(Clone)]
struct CountingModal {
    name: &'static str,
    hits: Arc<Mutex<Vec<&'static str>>>,
    size: ModalSizeConstraints,
    backdrop: BackdropConfig,
}

impl CountingModal {
    fn new(
        name: &'static str,
        hits: Arc<Mutex<Vec<&'static str>>>,
        backdrop: BackdropConfig,
    ) -> Self {
        Self {
            name,
            hits,
            size: ModalSizeConstraints::new()
                .min_width(20)
                .max_width(30)
                .min_height(6)
                .max_height(10),
            backdrop,
        }
    }
}

impl StackModal for CountingModal {
    fn render_content(&self, area: Rect, frame: &mut Frame) {
        if let Some(cell) = frame.buffer.get_mut(area.x, area.y) {
            cell.content =
                ftui_render::cell::CellContent::from_char(self.name.chars().next().unwrap_or('?'));
        }
    }

    fn handle_event(
        &mut self,
        _event: &Event,
        _hit: Option<(HitId, HitRegion, HitData)>,
        _hit_id: HitId,
    ) -> Option<ModalResultData> {
        if let Ok(mut hits) = self.hits.lock() {
            hits.push(self.name);
        }
        None
    }

    fn size_constraints(&self) -> ModalSizeConstraints {
        self.size
    }

    fn backdrop_config(&self) -> BackdropConfig {
        self.backdrop
    }
}

#[test]
fn modal_stack_input_isolated_to_top() {
    log_jsonl("env", &[("test", "modal_stack_input_isolated_to_top")]);
    let hits: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let mut stack = ModalStack::new();

    stack.push(Box::new(CountingModal::new(
        "bottom",
        hits.clone(),
        BackdropConfig::new(PackedRgba::rgb(32, 0, 0), 0.4),
    )));
    stack.push(Box::new(CountingModal::new(
        "top",
        hits.clone(),
        BackdropConfig::new(PackedRgba::rgb(0, 0, 48), 0.6),
    )));

    let _ = stack.handle_event(&press(KeyCode::Enter), None);
    let recorded = hits.lock().unwrap().clone();
    assert_eq!(recorded, vec!["top"]);
}

#[test]
fn modal_focus_trap_restores_previous_focus() {
    log_jsonl(
        "env",
        &[("test", "modal_focus_trap_restores_previous_focus")],
    );
    let mut modals = FocusAwareModalStack::new();

    // Seed focus graph and set initial focus.
    modals.with_focus_graph_mut(|graph| {
        for id in [1_u64, 2, 3] {
            graph.insert(FocusNode::new(id, Rect::new(0, 0, 1, 1)));
        }
    });
    modals.focus(3);
    assert_eq!(modals.focus_manager().current(), Some(3));

    // Push a modal with focus trap; should move focus to first item.
    modals.push_with_trap(
        Box::new(WidgetModalEntry::new(sample_content()).with_focusable_ids(vec![1, 2])),
        vec![1, 2],
    );
    assert_eq!(modals.focus_manager().current(), Some(1));

    // Close modal and ensure focus restores to prior target.
    let result = modals.handle_event(&press(KeyCode::Escape), None);
    assert!(result.is_some());
    assert!(matches!(
        result.unwrap().data,
        Some(ModalResultData::Dismissed)
    ));
    assert_eq!(modals.focus_manager().current(), Some(3));
}

#[test]
fn modal_focus_trap_non_lifo_close_restores_background_focus_e2e() {
    log_jsonl(
        "env",
        &[(
            "test",
            "modal_focus_trap_non_lifo_close_restores_background_focus_e2e",
        )],
    );
    let mut modals = FocusAwareModalStack::new();

    modals.with_focus_graph_mut(|graph| {
        for id in [1_u64, 2, 3] {
            graph.insert(FocusNode::new(id, Rect::new(0, 0, 1, 1)));
        }
    });
    modals.focus(3);

    let lower_id = modals.push_with_trap(
        Box::new(WidgetModalEntry::new(sample_content()).with_focusable_ids(vec![1])),
        vec![1],
    );
    modals.push_with_trap(
        Box::new(WidgetModalEntry::new(sample_content()).with_focusable_ids(vec![2])),
        vec![2],
    );

    let removed = modals.pop_id(lower_id);
    assert_eq!(removed.map(|result| result.id), Some(lower_id));
    assert_eq!(modals.focus_manager().current(), Some(2));
    assert!(modals.is_focus_trapped());

    let result = modals.handle_event(&press(KeyCode::Escape), None);
    assert!(result.is_some());
    assert_eq!(modals.focus_manager().current(), Some(3));
    assert!(!modals.is_focus_trapped());
}

#[test]
fn modal_custom_position_120x40() {
    let modal = Modal::new(sample_content())
        .size(
            ModalSizeConstraints::new()
                .min_width(40)
                .max_width(50)
                .min_height(12)
                .max_height(15),
        )
        .position(ModalPosition::Custom { x: 10, y: 5 });

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(120, 40, &mut pool);
    let area = Rect::new(0, 0, 120, 40);

    fill_background(&mut frame, area);
    modal.render(area, &mut frame);
    log_snapshot("modal_custom_position_120x40", &frame);
    assert_snapshot!("modal_custom_position_120x40", &frame.buffer);
}

#[test]
fn modal_zero_area_no_panic() {
    let modal = Modal::new(sample_content());

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(1, 1, &mut pool);
    let area = Rect::new(0, 0, 0, 0);

    // Should not panic
    modal.render(area, &mut frame);
}

#[test]
fn modal_content_rect_clamped_to_area() {
    let modal = Modal::new(sample_content())
        .size(ModalSizeConstraints::new().min_width(100).min_height(50))
        .position(ModalPosition::Custom { x: 200, y: 100 });

    let area = Rect::new(0, 0, 80, 24);
    let content_rect = modal.content_rect(area);

    // Content rect should be clamped within area
    assert!(content_rect.x >= area.x);
    assert!(content_rect.y >= area.y);
    assert!(content_rect.right() <= area.right());
    assert!(content_rect.bottom() <= area.bottom());
}

// ---------------------------------------------------------------------------
// Helper: Fill background with pattern to show backdrop effect
// ---------------------------------------------------------------------------

fn fill_background(frame: &mut Frame, area: Rect) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = frame.buffer.get_mut(x, y) {
                // Checkerboard pattern with dots and spaces
                let ch = if (x + y) % 2 == 0 { '.' } else { ' ' };
                cell.content = ftui_render::cell::CellContent::from_char(ch);
            }
        }
    }
}

// ===========================================================================
// Determinism tests
// ===========================================================================

#[test]
fn modal_render_is_deterministic() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn capture_hash(width: u16, height: u16) -> u64 {
        let modal = Modal::new(sample_content()).size(
            ModalSizeConstraints::new()
                .min_width(30)
                .max_width(40)
                .min_height(8)
                .max_height(10),
        );

        let mut pool = GraphemePool::new();
        let mut frame = Frame::new(width, height, &mut pool);
        let area = Rect::new(0, 0, width, height);
        fill_background(&mut frame, area);
        modal.render(area, &mut frame);

        let mut hasher = DefaultHasher::new();
        for y in 0..height {
            for x in 0..width {
                if let Some(cell) = frame.buffer.get(x, y)
                    && let Some(ch) = cell.content.as_char()
                {
                    ch.hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }

    // Multiple renders should produce identical output
    let hash1 = capture_hash(80, 24);
    let hash2 = capture_hash(80, 24);
    let hash3 = capture_hash(80, 24);

    assert_eq!(hash1, hash2, "Modal render should be deterministic");
    assert_eq!(hash2, hash3, "Modal render should be deterministic");
}

// ===========================================================================
// Animation Tests (bd-39vx.4)
// ===========================================================================

use ftui_widgets::modal::{
    ModalAnimationConfig, ModalAnimationPhase, ModalAnimationState, ModalEasing,
    ModalEntranceAnimation, ModalExitAnimation,
};
use std::time::Duration;

#[test]
fn animation_opening_sequence() {
    let mut state = ModalAnimationState::new();
    let config = ModalAnimationConfig::default();

    // Initially closed
    assert_eq!(state.phase(), ModalAnimationPhase::Closed);
    assert!(!state.is_visible());

    // Start opening
    state.start_opening();
    assert_eq!(state.phase(), ModalAnimationPhase::Opening);
    assert!(state.is_visible());

    // Advance animation
    state.tick(Duration::from_millis(100), &config);
    assert!(state.is_animating());

    // Values should be interpolating
    let scale = state.current_scale(&config);
    let opacity = state.current_opacity(&config);
    assert!(scale > config.min_scale);
    assert!(scale < 1.0);
    assert!(opacity > 0.0);
    assert!(opacity < 1.0);

    // Complete animation
    state.tick(Duration::from_millis(200), &config);
    assert_eq!(state.phase(), ModalAnimationPhase::Open);
    assert!((state.current_scale(&config) - 1.0).abs() < 0.001);
    assert!((state.current_opacity(&config) - 1.0).abs() < 0.001);
}

#[test]
fn animation_closing_sequence() {
    let mut state = ModalAnimationState::open();
    let config = ModalAnimationConfig::default();

    // Start closing
    state.start_closing();
    assert_eq!(state.phase(), ModalAnimationPhase::Closing);

    // Advance halfway
    state.tick(Duration::from_millis(75), &config);
    let scale = state.current_scale(&config);
    let opacity = state.current_opacity(&config);
    assert!(scale > config.min_scale);
    assert!(scale < 1.0);
    assert!(opacity > 0.0);
    assert!(opacity < 1.0);

    // Complete
    state.tick(Duration::from_millis(200), &config);
    assert_eq!(state.phase(), ModalAnimationPhase::Closed);
    assert!(!state.is_visible());
}

#[test]
fn animation_rapid_toggle_cancellation() {
    let mut state = ModalAnimationState::new();
    let config = ModalAnimationConfig::default();

    // Start opening
    state.start_opening();
    state.tick(Duration::from_millis(50), &config); // 25% through

    let opening_progress = state.progress();
    assert!(opening_progress > 0.0);
    assert!(opening_progress < 0.5);

    // Rapidly close - should reverse direction
    state.start_closing();
    assert_eq!(state.phase(), ModalAnimationPhase::Closing);

    // Progress should be inverted (if we were 25% open, now 75% through closing)
    let closing_progress = state.progress();
    assert!((opening_progress + closing_progress - 1.0).abs() < 0.001);

    // Rapidly open again - should reverse again
    state.start_opening();
    assert_eq!(state.phase(), ModalAnimationPhase::Opening);

    // Should resume from where we were in the opening direction
    let reopening_progress = state.progress();
    assert!((reopening_progress - opening_progress).abs() < 0.001);
}

#[test]
fn animation_reduced_motion_disables_scale() {
    let mut state = ModalAnimationState::new();
    state.set_reduced_motion(true);

    let config = ModalAnimationConfig::default();

    // With reduced motion, scale should always be 1.0
    state.start_opening();
    state.tick(Duration::from_millis(50), &config);

    let scale = state.current_scale(&config);
    assert!(
        (scale - 1.0).abs() < 0.001,
        "Reduced motion should disable scale animation"
    );

    // But opacity should still animate
    let opacity = state.current_opacity(&config);
    assert!(opacity < 1.0, "Opacity should still animate");
}

#[test]
fn animation_reduced_motion_uses_fade() {
    let config = ModalAnimationConfig::reduced_motion();

    assert!(matches!(config.entrance, ModalEntranceAnimation::FadeIn));
    assert!(matches!(config.exit, ModalExitAnimation::FadeOut));
    assert!((config.min_scale - 1.0).abs() < 0.001);
}

#[test]
fn animation_easing_functions_bounded() {
    // Test all easing functions stay within bounds at key points
    let easings = [
        ModalEasing::Linear,
        ModalEasing::EaseIn,
        ModalEasing::EaseOut,
        ModalEasing::EaseInOut,
    ];

    for easing in easings {
        assert_eq!(easing.apply(0.0), 0.0, "{:?} at 0", easing);
        assert_eq!(easing.apply(1.0), 1.0, "{:?} at 1", easing);

        // Mid-point should be in (0, 1)
        let mid = easing.apply(0.5);
        assert!(mid > 0.0 && mid < 1.0, "{:?} at 0.5 = {}", easing, mid);
    }

    // Back easing can overshoot (it's the only one that should)
    assert!(ModalEasing::Back.can_overshoot());
    assert!(!ModalEasing::EaseOut.can_overshoot());
}

#[test]
fn animation_backdrop_animates_independently() {
    let mut state = ModalAnimationState::new();
    let config = ModalAnimationConfig::default()
        .entrance_duration(Duration::from_millis(200))
        .backdrop_duration(Duration::from_millis(100));

    state.start_opening();

    // After 100ms, backdrop should be complete but content still animating
    state.tick(Duration::from_millis(100), &config);

    let content_progress = state.progress();
    let backdrop_progress = state.backdrop_progress();

    // Backdrop animates faster
    assert!(backdrop_progress > content_progress);
    assert!((backdrop_progress - 1.0).abs() < 0.001);
    assert!(content_progress < 1.0);
}

#[test]
fn animation_force_open_skips_animation() {
    let mut state = ModalAnimationState::new();

    state.force_open();

    assert_eq!(state.phase(), ModalAnimationPhase::Open);
    assert!(!state.is_animating());
    assert_eq!(state.progress(), 1.0);
}

#[test]
fn animation_force_close_skips_animation() {
    let mut state = ModalAnimationState::open();

    state.force_close();

    assert_eq!(state.phase(), ModalAnimationPhase::Closed);
    assert!(!state.is_animating());
    assert_eq!(state.progress(), 0.0);
}

#[test]
fn animation_slide_down_y_offset() {
    let mut state = ModalAnimationState::new();
    let config = ModalAnimationConfig::default().entrance(ModalEntranceAnimation::SlideDown);

    state.start_opening();

    // Initially should have negative Y offset (above final position)
    let initial_offset = state.current_y_offset(&config, 10);
    assert!(
        initial_offset < 0,
        "Initial offset should be negative for slide-down"
    );

    // After animation completes, offset should be 0
    state.tick(Duration::from_millis(500), &config);
    let final_offset = state.current_y_offset(&config, 10);
    assert_eq!(final_offset, 0);
}

#[test]
fn animation_determinism() {
    // Same inputs should produce same outputs
    let config = ModalAnimationConfig::default();

    let mut state1 = ModalAnimationState::new();
    let mut state2 = ModalAnimationState::new();

    state1.start_opening();
    state2.start_opening();

    for _ in 0..10 {
        state1.tick(Duration::from_millis(20), &config);
        state2.tick(Duration::from_millis(20), &config);

        assert_eq!(state1.phase(), state2.phase());
        assert!((state1.progress() - state2.progress()).abs() < f64::EPSILON);
        assert!((state1.current_scale(&config) - state2.current_scale(&config)).abs() < 0.001);
        assert!((state1.current_opacity(&config) - state2.current_opacity(&config)).abs() < 0.001);
    }
}
