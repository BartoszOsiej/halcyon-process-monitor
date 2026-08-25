#![forbid(unsafe_code)]

//! Focus-aware modal integration for automatic focus trap management.
//!
//! This module provides `FocusAwareModalStack`, which combines [`ModalStack`]
//! with [`FocusManager`] integration for automatic focus trapping when modals
//! are opened and focus restoration when they close.
//!
//! # Invariants
//!
//! 1. **Auto-focus**: When a modal opens with a focus group, focus moves to the
//!    first focusable element in that group.
//! 2. **Focus trap**: Tab navigation is constrained to the modal's focus group.
//! 3. **Focus restoration**: When a modal closes, focus returns to where it was
//!    before the modal opened.
//! 4. **LIFO ordering**: Focus traps follow modal stack ordering (nested modals
//!    restore to the correct previous state).
//!
//! # Failure Modes
//!
//! - If the focus group has no focusable members, focus remains unchanged.
//! - If the original focus target is removed during modal display, focus moves
//!   to the first available focusable element.
//! - Focus trap with an empty group allows focus to escape (graceful degradation).
//!
//! # Example
//!
//! ```ignore
//! use ftui_widgets::focus::FocusManager;
//! use ftui_widgets::modal::{ModalStack, WidgetModalEntry};
//! use ftui_widgets::modal::focus_integration::FocusAwareModalStack;
//!
//! let mut modals = FocusAwareModalStack::new();
//!
//! // Push modal with focus group members
//! let focus_ids = vec![ok_button_id, cancel_button_id];
//! let modal_id = modals.push_with_trap(
//!     Box::new(WidgetModalEntry::new(dialog)),
//!     focus_ids,
//! );
//!
//! // Handle event (focus trap active, Escape closes and restores focus)
//! if let Some(result) = modals.handle_event(&event, None) {
//!     // Modal closed, focus already restored
//! }
//! ```

use std::sync::atomic::{AtomicU32, Ordering};

use ftui_core::event::Event;
use ftui_core::geometry::Rect;
use ftui_render::frame::{Frame, HitTestResult};

use crate::focus::{FocusId, FocusManager};
use crate::modal::stack::FocusTrapSpec;
use crate::modal::{ModalId, ModalResult, ModalStack, StackModal};

/// Global counter for unique focus group IDs.
static FOCUS_GROUP_COUNTER: AtomicU32 = AtomicU32::new(1_000_000);

/// Generate a unique focus group ID.
pub(super) fn next_focus_group_id(focus_manager: &FocusManager) -> u32 {
    loop {
        let group_id = FOCUS_GROUP_COUNTER.fetch_add(1, Ordering::Relaxed);
        if !focus_manager.has_group(group_id) {
            return group_id;
        }
    }
}

pub(super) struct ModalFocusCoordinator<'a> {
    stack: &'a mut ModalStack,
    focus_manager: &'a mut FocusManager,
    base_focus: &'a mut Option<Option<FocusId>>,
}

impl<'a> ModalFocusCoordinator<'a> {
    pub(super) fn new(
        stack: &'a mut ModalStack,
        focus_manager: &'a mut FocusManager,
        base_focus: &'a mut Option<Option<FocusId>>,
    ) -> Self {
        Self {
            stack,
            focus_manager,
            base_focus,
        }
    }

    pub(super) fn push_modal_with_trap<F>(
        &mut self,
        modal: Box<dyn StackModal>,
        focusable_ids: Option<Vec<FocusId>>,
        trap_enabled: bool,
        allocate_group_id: F,
    ) -> ModalId
    where
        F: FnOnce(&FocusManager) -> u32,
    {
        let base_focus = if self.focus_manager.host_focused() {
            self.focus_manager.current()
        } else {
            self.focus_manager.deferred_focus_target()
        };
        let was_trapped = self.focus_manager.is_trapped();
        let focus_group_id = if trap_enabled {
            if let Some(ids) = focusable_ids {
                let group_id = allocate_group_id(self.focus_manager);
                let has_declared_members = !ids.is_empty();
                self.focus_manager
                    .create_group_preserving_members(group_id, ids);
                let trapped = self.focus_manager.push_trap(group_id);
                if !trapped && !has_declared_members {
                    self.focus_manager.remove_group(group_id);
                    None
                } else {
                    if !was_trapped && trapped {
                        *self.base_focus = Some(base_focus);
                    }
                    Some(group_id)
                }
            } else {
                None
            }
        } else {
            None
        };

        let modal_id = self.stack.push_with_focus(modal, focus_group_id);
        if focus_group_id.is_some() {
            let _ = self.stack.set_focus_return_focus(modal_id, base_focus);
        }
        modal_id
    }

    pub(super) fn pop_modal(&mut self) -> Option<ModalResult> {
        let result = self.stack.pop()?;
        self.handle_closed_result(&result);
        Some(result)
    }

    pub(super) fn pop_modal_by_id(&mut self, id: ModalId) -> Option<ModalResult> {
        if self.stack.top_id() == Some(id) {
            return self.pop_modal();
        }

        if let Some(group_id) = self.stack.focus_group_id(id) {
            let removed_members = self.focus_manager.group_members(group_id);
            let removed_group_active = self.group_has_focusable_member(group_id);
            let removed_effective_return_focus = self
                .effective_focus_return_focuses_in_order_skipping(None)
                .into_iter()
                .find_map(|(candidate_group_id, return_focus)| {
                    (candidate_group_id == group_id).then_some(return_focus)
                });

            if let Some((upper_modal_id, _)) = self.stack.next_focus_modal_after(id) {
                let should_retarget = if removed_group_active {
                    true
                } else {
                    let upper_return_focus = self
                        .stack
                        .focus_modal_specs_in_order()
                        .into_iter()
                        .find_map(|(modal_id, trap)| {
                            (modal_id == upper_modal_id).then_some(trap.return_focus)
                        })
                        .flatten();
                    !self.return_focus_remains_valid_after_removing_group(
                        upper_modal_id,
                        upper_return_focus,
                        group_id,
                        &removed_members,
                    )
                };

                if should_retarget && let Some(return_focus) = removed_effective_return_focus {
                    let _ = self
                        .stack
                        .set_focus_return_focus(upper_modal_id, return_focus);
                }
            }
        }

        let result = self.stack.pop_id_with_restore_retarget(id, false)?;
        if let Some(group_id) = result.focus_group_id {
            let closing_members = self.focus_manager.group_members(group_id);
            self.focus_manager.remove_group_without_repair(group_id);
            self.focus_manager
                .clear_deferred_focus_if_excluded(&closing_members);
            self.rebuild_focus_traps();
            self.focus_manager
                .repair_focus_after_excluding_ids(&closing_members);
            self.refresh_inactive_modal_return_focus_targets();
        }
        Some(result)
    }

    pub(super) fn pop_all_modals(&mut self) -> Vec<ModalResult> {
        let results = self.stack.pop_all();
        let mut removed_group = false;
        let mut removed_members = Vec::new();
        for result in &results {
            if let Some(group_id) = result.focus_group_id {
                removed_members.extend(self.focus_manager.group_members(group_id));
                self.focus_manager.remove_group_without_repair(group_id);
                removed_group = true;
            }
        }
        if removed_group {
            self.focus_manager
                .clear_deferred_focus_if_excluded(&removed_members);
            self.rebuild_focus_traps();
            self.focus_manager
                .repair_focus_after_excluding_ids(&removed_members);
            self.refresh_inactive_modal_return_focus_targets();
        }
        results
    }

    pub(super) fn handle_modal_event(
        &mut self,
        event: &Event,
        hit: Option<HitTestResult>,
    ) -> Option<ModalResult> {
        if let Event::Focus(focused) = event {
            if *focused && self.stack.is_empty() && self.base_focus.is_some() {
                let deferred_focus = self.focus_manager.deferred_focus_target();
                self.focus_manager.set_host_focused(true);
                if let Some(id) = deferred_focus {
                    *self.base_focus = Some(Some(id));
                }
                self.rebuild_focus_traps();
            } else {
                self.focus_manager.apply_host_focus(*focused);
            }
            if *focused {
                self.refresh_inactive_modal_return_focus_targets();
            }
        }
        let result = self.stack.handle_event(event, hit)?;
        self.handle_closed_result(&result);
        Some(result)
    }

    pub(super) fn rebuild_focus_traps(&mut self) {
        let (trap_specs, trailing_failed_restore) = self.collapsed_focus_trap_specs();
        let had_active_trap_before = self.focus_manager.is_trapped();
        let preserved_logical_target = self.focus_manager.logical_focus_target();
        let activation_base_focus = if self.focus_manager.host_focused() {
            self.focus_manager.current()
        } else {
            self.focus_manager.deferred_focus_target()
        };
        self.focus_manager.clear_traps();

        if !self.focus_manager.host_focused() {
            let mut has_active_trap = false;
            if self.focus_manager.current().is_some() {
                let _ = self.focus_manager.blur();
            }

            for trap in trap_specs.iter().copied() {
                has_active_trap |= self
                    .focus_manager
                    .push_trap_with_return_focus(trap.group_id, trap.return_focus);
            }

            if has_active_trap && !had_active_trap_before && self.base_focus.is_none() {
                *self.base_focus = Some(activation_base_focus);
            }

            if !has_active_trap {
                let restore_target = (!had_active_trap_before)
                    .then_some(preserved_logical_target)
                    .flatten()
                    .map(Some)
                    .or(trailing_failed_restore)
                    .or(*self.base_focus);
                if let Some(target) = restore_target {
                    self.focus_manager.replace_deferred_focus_target(target);
                }
            } else if self.focus_manager.logical_focus_target().is_none()
                && let Some(target) = trailing_failed_restore
            {
                self.focus_manager.replace_deferred_focus_target(target);
            }
            return;
        }

        let mut has_active_trap = false;
        for trap in trap_specs.iter().copied() {
            has_active_trap |= self
                .focus_manager
                .push_trap_with_return_focus(trap.group_id, trap.return_focus);
        }

        if has_active_trap && !had_active_trap_before && self.base_focus.is_none() {
            *self.base_focus = Some(activation_base_focus);
        }

        if !has_active_trap {
            let restore_target = (!had_active_trap_before)
                .then_some(preserved_logical_target)
                .flatten()
                .map(Some)
                .or(trailing_failed_restore)
                .or(*self.base_focus);
            match restore_target {
                Some(Some(base_focus)) => {
                    let _ = self.focus_manager.focus_without_history(base_focus);
                }
                Some(None) if self.focus_manager.current().is_some() => {
                    let _ = self.focus_manager.blur();
                }
                Some(None) => {}
                None => {}
            }

            if matches!(restore_target, Some(Some(base_focus)) if self.focus_manager.current() != Some(base_focus))
            {
                self.focus_manager.focus_first_without_history_for_restore();
            }
            if self.focus_manager.current().is_some_and(|id| {
                self.focus_manager
                    .graph()
                    .get(id)
                    .map(|node| !node.is_focusable)
                    .unwrap_or(true)
            }) {
                let _ = self.focus_manager.blur();
            }
            *self.base_focus = None;
            return;
        }

        if self.focus_manager.logical_focus_target().is_none()
            && let Some(Some(target)) = trailing_failed_restore
        {
            let _ = self.focus_manager.focus_without_history(target);
        }

        let _ = self.focus_manager.apply_host_focus(true);
    }

    fn return_focus_remains_valid_after_removing_group(
        &self,
        upper_modal_id: ModalId,
        return_focus: Option<FocusId>,
        removed_group_id: u32,
        removed_members: &[FocusId],
    ) -> bool {
        let mut surviving_lower_active_group = None;
        for (modal_id, trap) in self.stack.focus_modal_specs_in_order() {
            if modal_id == upper_modal_id {
                break;
            }
            if trap.group_id == removed_group_id {
                continue;
            }
            if self.group_has_focusable_member(trap.group_id) {
                surviving_lower_active_group = Some(trap.group_id);
            }
        }

        if let Some(group_id) = surviving_lower_active_group {
            return self.focus_target_in_group(return_focus, group_id);
        }

        match return_focus {
            None => true,
            Some(id) => self.focus_target_is_focusable(Some(id)) && !removed_members.contains(&id),
        }
    }

    fn effective_focus_return_focuses_in_order_skipping(
        &self,
        skipped_modal_id: Option<ModalId>,
    ) -> Vec<(u32, Option<FocusId>)> {
        let mut effective = Vec::new();
        let mut lower_active_group = None;
        let mut lower_fallback_return_focus = None;

        for (_, trap) in self
            .stack
            .focus_modal_specs_in_order()
            .into_iter()
            .filter(|(modal_id, _)| Some(*modal_id) != skipped_modal_id)
        {
            let effective_return_focus = if let Some(group_id) = lower_active_group {
                if self.focus_target_in_group(trap.return_focus, group_id) {
                    trap.return_focus
                } else {
                    lower_fallback_return_focus.unwrap_or(trap.return_focus)
                }
            } else if self.focus_target_is_focusable(trap.return_focus) {
                trap.return_focus
            } else {
                lower_fallback_return_focus.unwrap_or(trap.return_focus)
            };

            effective.push((trap.group_id, effective_return_focus));
            lower_fallback_return_focus = Some(effective_return_focus);
            if self.group_has_focusable_member(trap.group_id) {
                lower_active_group = Some(trap.group_id);
            }
        }

        effective
    }

    fn collapsed_focus_trap_specs(&self) -> (Vec<FocusTrapSpec>, Option<Option<FocusId>>) {
        let mut collapsed = Vec::new();
        let mut trailing_failed_restore = None;

        for (group_id, effective_return_focus) in
            self.effective_focus_return_focuses_in_order_skipping(None)
        {
            if self.group_has_focusable_member(group_id) {
                collapsed.push(FocusTrapSpec {
                    group_id,
                    return_focus: effective_return_focus,
                });
                trailing_failed_restore = None;
            } else {
                trailing_failed_restore = Some(effective_return_focus);
            }
        }

        (collapsed, trailing_failed_restore)
    }

    fn group_has_focusable_member(&self, group_id: u32) -> bool {
        self.focus_manager
            .group_members(group_id)
            .into_iter()
            .any(|id| self.focus_target_is_focusable(Some(id)))
    }

    fn focus_target_is_focusable(&self, target: Option<FocusId>) -> bool {
        target.is_some_and(|id| {
            self.focus_manager
                .graph()
                .get(id)
                .map(|node| node.is_focusable)
                .unwrap_or(false)
        })
    }

    fn focus_target_in_group(&self, target: Option<FocusId>, group_id: u32) -> bool {
        let Some(target) = target else {
            return false;
        };
        self.focus_target_is_focusable(Some(target))
            && self.focus_manager.group_members(group_id).contains(&target)
    }

    fn handle_closed_result(&mut self, result: &ModalResult) {
        if let Some(group_id) = result.focus_group_id {
            self.close_focus_group(group_id);
        }
    }

    fn close_focus_group(&mut self, group_id: u32) {
        let closing_members = self.focus_manager.group_members(group_id);
        if self.group_has_focusable_member(group_id) {
            self.focus_manager.pop_trap();
            self.focus_manager.remove_group(group_id);
        } else {
            self.focus_manager.remove_group_without_repair(group_id);
        }
        self.focus_manager
            .repair_focus_after_excluding_ids(&closing_members);
        if !self.focus_manager.is_trapped() && self.focus_manager.host_focused() {
            *self.base_focus = None;
        }
        self.refresh_inactive_modal_return_focus_targets();
    }

    pub(super) fn refresh_inactive_modal_return_focus_targets(&mut self) {
        let logical_target = self.focus_manager.logical_focus_target();
        let focus_modals = self.stack.focus_modal_specs_in_order();

        let topmost_active_index = focus_modals
            .iter()
            .rposition(|(_, trap)| self.group_has_focusable_member(trap.group_id));

        let start_index = topmost_active_index.map_or(0, |index| index + 1);
        for (modal_id, trap) in focus_modals.into_iter().skip(start_index) {
            if self.group_has_focusable_member(trap.group_id) {
                continue;
            }
            let _ = self.stack.set_focus_return_focus(modal_id, logical_target);
        }

        self.refresh_active_modal_return_focus_targets_for_invalid_lower_selections(
            &self.stack.focus_modal_specs_in_order(),
            topmost_active_index,
        );
    }

    fn refresh_active_modal_return_focus_targets_for_invalid_lower_selections(
        &mut self,
        focus_modals: &[(ModalId, FocusTrapSpec)],
        topmost_active_index: Option<usize>,
    ) {
        let Some(topmost_active_index) = topmost_active_index else {
            return;
        };

        for upper_idx in 1..=topmost_active_index {
            let (_, lower_trap) = focus_modals[upper_idx - 1];
            let (upper_modal_id, upper_trap) = focus_modals[upper_idx];

            if !self.group_has_focusable_member(lower_trap.group_id)
                || self.focus_target_in_group(upper_trap.return_focus, lower_trap.group_id)
            {
                continue;
            }

            let replacement = self.first_focusable_in_group(lower_trap.group_id);
            let _ = self
                .stack
                .set_focus_return_focus(upper_modal_id, replacement);
        }
    }

    fn first_focusable_in_group(&self, group_id: u32) -> Option<FocusId> {
        self.focus_manager.group_primary_focus_target(group_id)
    }
}

/// Modal stack with integrated focus management.
///
/// This wrapper provides automatic focus trapping when modals open and
/// focus restoration when they close. It manages both the modal stack
/// and focus manager in a coordinated way.
///
/// # Invariants
///
/// - Focus trap stack depth equals the number of modals with focus groups.
/// - Each modal's focus group ID is unique and not reused.
/// - Pop operations always call `pop_trap` for modals with focus groups.
pub struct FocusAwareModalStack {
    stack: ModalStack,
    focus_manager: FocusManager,
    base_focus: Option<Option<FocusId>>,
}

impl Default for FocusAwareModalStack {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusAwareModalStack {
    /// Create a new focus-aware modal stack.
    pub fn new() -> Self {
        Self {
            stack: ModalStack::new(),
            focus_manager: FocusManager::new(),
            base_focus: None,
        }
    }

    /// Create from existing stack and focus manager.
    ///
    /// Use this when you already have a `FocusManager` in your application
    /// and want to integrate modal focus trapping.
    ///
    /// The provided manager must not already have active modal traps. This
    /// wrapper only tracks traps for modals it owns, so starting from an
    /// already-trapped manager would make later rebuild/pop operations
    /// silently corrupt unrelated trap state.
    pub fn with_focus_manager(focus_manager: FocusManager) -> Self {
        assert!(
            !focus_manager.is_trapped(),
            "FocusAwareModalStack requires a FocusManager without active traps",
        );
        Self {
            stack: ModalStack::new(),
            focus_manager,
            base_focus: None,
        }
    }

    // --- Modal Stack Delegation ---

    /// Push a modal without focus trapping.
    ///
    /// The modal will be rendered and receive events, but focus is not managed.
    pub fn push(&mut self, modal: Box<dyn StackModal>) -> ModalId {
        self.stack.push(modal)
    }

    /// Push a modal with automatic focus trapping.
    ///
    /// # Parameters
    /// - `modal`: The modal content
    /// - `focusable_ids`: The focus IDs of elements inside the modal
    ///
    /// # Behavior
    /// 1. Creates a focus group with the provided IDs
    /// 2. Pushes a focus trap (saving current focus)
    /// 3. Moves focus to the first element in the group
    pub fn push_with_trap(
        &mut self,
        modal: Box<dyn StackModal>,
        focusable_ids: Vec<FocusId>,
    ) -> ModalId {
        ModalFocusCoordinator::new(
            &mut self.stack,
            &mut self.focus_manager,
            &mut self.base_focus,
        )
        .push_modal_with_trap(modal, Some(focusable_ids), true, next_focus_group_id)
    }

    /// Pop the top modal.
    ///
    /// If the modal had a focus group, the focus trap is popped and
    /// focus is restored to where it was before the modal opened.
    pub fn pop(&mut self) -> Option<ModalResult> {
        ModalFocusCoordinator::new(
            &mut self.stack,
            &mut self.focus_manager,
            &mut self.base_focus,
        )
        .pop_modal()
    }

    /// Pop a specific modal by ID.
    ///
    pub fn pop_id(&mut self, id: ModalId) -> Option<ModalResult> {
        ModalFocusCoordinator::new(
            &mut self.stack,
            &mut self.focus_manager,
            &mut self.base_focus,
        )
        .pop_modal_by_id(id)
    }

    /// Pop all modals, restoring focus to the original state.
    pub fn pop_all(&mut self) -> Vec<ModalResult> {
        ModalFocusCoordinator::new(
            &mut self.stack,
            &mut self.focus_manager,
            &mut self.base_focus,
        )
        .pop_all_modals()
    }

    /// Handle an event, routing to the top modal.
    ///
    /// If the modal closes (via Escape, backdrop click, etc.), the focus
    /// trap is automatically popped and focus is restored. For mouse events,
    /// pass the provenance-aware result from [`Frame::hit_test_detailed`].
    pub fn handle_event(
        &mut self,
        event: &Event,
        hit: Option<HitTestResult>,
    ) -> Option<ModalResult> {
        ModalFocusCoordinator::new(
            &mut self.stack,
            &mut self.focus_manager,
            &mut self.base_focus,
        )
        .handle_modal_event(event, hit)
    }

    /// Render all modals.
    pub fn render(&self, frame: &mut Frame, screen: Rect) {
        self.stack.render(frame, screen);
    }

    /// Perform a direct focus-graph mutation and automatically resynchronize modal focus state.
    pub fn with_focus_graph_mut<R>(
        &mut self,
        f: impl FnOnce(&mut crate::focus::FocusGraph) -> R,
    ) -> R {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f(self.focus_manager.graph_mut())
        }));
        let had_invalid_current = self.focus_manager.current().is_some_and(|id| {
            self.focus_manager
                .graph()
                .get(id)
                .map(|node| !node.is_focusable)
                .unwrap_or(true)
        });
        self.resync_focus_state();
        let needs_post_resync_restore = had_invalid_current
            && self.focus_manager.host_focused()
            && self.focus_manager.current().is_none_or(|id| {
                self.focus_manager
                    .graph()
                    .get(id)
                    .map(|node| !node.is_focusable)
                    .unwrap_or(true)
            });
        if needs_post_resync_restore {
            self.focus_manager.restore_focus_after_invalid_current();
            self.resync_inactive_modal_return_focus_targets();
        }
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Focus a specific target through the wrapped focus manager.
    pub fn focus(&mut self, id: FocusId) -> Option<FocusId> {
        let previous = self.focus_manager.focus(id);
        if previous.is_some()
            || self.focus_manager.current() == Some(id)
            || self.focus_manager.logical_focus_target() == Some(id)
        {
            self.resync_inactive_modal_return_focus_targets();
        }
        previous
    }

    fn resync_focus_state(&mut self) {
        let mut coordinator = ModalFocusCoordinator::new(
            &mut self.stack,
            &mut self.focus_manager,
            &mut self.base_focus,
        );
        coordinator.rebuild_focus_traps();
        coordinator.refresh_inactive_modal_return_focus_targets();
    }

    fn resync_inactive_modal_return_focus_targets(&mut self) {
        ModalFocusCoordinator::new(
            &mut self.stack,
            &mut self.focus_manager,
            &mut self.base_focus,
        )
        .refresh_inactive_modal_return_focus_targets();
    }

    // --- State Queries ---

    /// Check if the modal stack is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Get the number of open modals.
    #[inline]
    pub fn depth(&self) -> usize {
        self.stack.depth()
    }

    /// Check if focus is currently trapped in a modal.
    #[inline]
    pub fn is_focus_trapped(&self) -> bool {
        self.focus_manager.is_trapped()
    }

    /// Get a reference to the underlying modal stack.
    pub fn stack(&self) -> &ModalStack {
        &self.stack
    }

    /// Get a reference to the focus manager.
    pub fn focus_manager(&self) -> &FocusManager {
        &self.focus_manager
    }

    #[cfg(test)]
    fn stack_mut(&mut self) -> &mut ModalStack {
        &mut self.stack
    }

    #[cfg(test)]
    fn focus_manager_mut(&mut self) -> &mut FocusManager {
        &mut self.focus_manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Widget;
    use crate::focus::FocusNode;
    use crate::modal::WidgetModalEntry;
    use ftui_core::event::{KeyCode, KeyEvent, KeyEventKind, Modifiers};
    use ftui_core::geometry::Rect;

    #[derive(Debug, Clone)]
    struct StubWidget;

    impl Widget for StubWidget {
        fn render(&self, _area: Rect, _frame: &mut Frame) {}
    }

    fn make_focus_node(id: FocusId) -> FocusNode {
        FocusNode::new(id, Rect::new(0, 0, 10, 3)).with_tab_index(id as i32)
    }

    #[test]
    fn push_with_trap_creates_focus_trap() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(3));

        // Focus node 3 before opening modal
        modals.focus_manager_mut().focus(3);
        assert_eq!(modals.focus_manager().current(), Some(3));

        // Push modal with trap containing nodes 1 and 2
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1, 2]);

        // Focus should now be on node 1 (first in group)
        assert!(modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn pop_restores_focus() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(3));

        // Focus node 3 before opening modal
        modals.focus_manager_mut().focus(3);

        // Push modal with trap
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1, 2]);
        assert_eq!(modals.focus_manager().current(), Some(1));

        // Pop modal - focus should return to node 3
        modals.pop();
        assert!(!modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(3));
    }

    #[test]
    fn pop_discards_closed_modal_focus_history() {
        let mut modals = FocusAwareModalStack::new();
        for id in 1..=3 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        assert_eq!(modals.focus(3), Some(2));

        let result = modals.pop();
        assert!(result.is_some());
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.focus_manager_mut().focus_back());
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn pop_skips_closed_modal_focus_ids_when_background_focus_disappears() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(50));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(100));

        modals.focus_manager_mut().focus(100);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        let _ = modals.focus_manager_mut().graph_mut().remove(100);

        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(50));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn nested_modals_restore_correctly() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        for id in 1..=6 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        // Initial focus
        modals.focus_manager_mut().focus(1);

        // First modal traps to nodes 2, 3
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        assert_eq!(modals.focus_manager().current(), Some(2));

        // Second modal traps to nodes 4, 5, 6
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 5, 6]);
        assert_eq!(modals.focus_manager().current(), Some(4));

        // Pop second modal - back to first modal's focus (node 2)
        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(2));

        // Pop first modal - back to original focus (node 1)
        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_restores_none_when_modal_opened_without_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        assert_eq!(modals.focus_manager().current(), Some(1));

        modals.pop();
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn resync_focus_state_recovers_after_manual_stack_mutation() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(100));

        modals.focus_manager_mut().focus(100);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1, 2]);
        assert!(modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(1));

        let result = modals.stack_mut().pop();
        assert!(result.is_some());
        assert!(modals.is_focus_trapped());

        modals.resync_focus_state();
        assert!(!modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(100));
    }

    #[test]
    fn handle_event_escape_restores_focus() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));

        // Focus node 2
        modals.focus_manager_mut().focus(2);

        // Push modal
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        assert_eq!(modals.focus_manager().current(), Some(1));

        // Escape closes modal
        let escape = Event::Key(KeyEvent {
            code: KeyCode::Escape,
            modifiers: Modifiers::empty(),
            kind: KeyEventKind::Press,
        });

        let result = modals.handle_event(&escape, None);
        assert!(result.is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
    }

    #[test]
    fn handle_event_focus_loss_blurs_current_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals.focus_manager_mut().focus(1);
        let _ = modals.focus_manager_mut().take_focus_event();

        let result = modals.handle_event(&Event::Focus(false), None);
        assert!(result.is_none());
        assert_eq!(modals.focus_manager().current(), None);
        assert_eq!(
            modals.focus_manager_mut().take_focus_event(),
            Some(crate::focus::FocusEvent::FocusLost { id: 1 })
        );
    }

    #[test]
    fn handle_event_focus_gain_restores_trapped_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(3));
        modals.focus_manager_mut().focus(3);

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1, 2]);
        assert_eq!(modals.focus_manager().current(), Some(1));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        let result = modals.handle_event(&Event::Focus(true), None);
        assert!(result.is_none());
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn push_with_trap_autofocuses_negative_tabindex_member_when_modal_has_no_tabbable_nodes() {
        let mut modals = FocusAwareModalStack::new();

        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(FocusNode::new(2, Rect::new(0, 0, 10, 3)).with_tab_index(-1));
        modals.focus_manager_mut().focus(1);

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);

        assert!(modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(2));
    }

    #[test]
    fn push_with_trap_blurred_restores_negative_tabindex_member_on_focus_gain() {
        let mut modals = FocusAwareModalStack::new();

        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(FocusNode::new(2, Rect::new(0, 0, 10, 3)).with_tab_index(-1));
        modals.focus_manager_mut().focus(1);
        let _ = modals.handle_event(&Event::Focus(false), None);

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);

        assert!(modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), None);

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(2));
    }

    #[test]
    fn push_without_trap_no_focus_change() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));

        // Focus node 2
        modals.focus_manager_mut().focus(2);

        // Push modal without trap
        modals.push(Box::new(WidgetModalEntry::new(StubWidget)));

        // Focus should not change
        assert!(!modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(2));
    }

    #[test]
    fn pop_all_restores_all_focus() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        for id in 1..=4 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        // Initial focus
        modals.focus_manager_mut().focus(1);

        // Push multiple modals
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![3]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        assert_eq!(modals.depth(), 3);
        assert_eq!(modals.focus_manager().current(), Some(4));

        // Pop all
        let results = modals.pop_all();
        assert_eq!(results.len(), 3);
        assert!(modals.is_empty());
        assert!(!modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn pop_all_restores_base_focus_without_intermediate_hop() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 5]);
        modals.focus(5);
        let _ = modals.focus_manager_mut().take_focus_event();
        let before = modals.focus_manager().focus_change_count();

        let results = modals.pop_all();

        assert_eq!(results.len(), 2);
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert_eq!(
            modals.focus_manager_mut().take_focus_event(),
            Some(crate::focus::FocusEvent::FocusMoved { from: 5, to: 1 })
        );
        assert_eq!(modals.focus_manager().focus_change_count(), before + 1);
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_restores_none_when_last_modal_opened_without_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));

        let modal_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        assert_eq!(modals.focus_manager().current(), Some(1));

        let _ = modals.pop_id(modal_id);
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_rebuild_preserves_unfocused_base_state_for_remaining_modal() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));

        let lower_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        let upper_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), Some(2));

        let removed = modals.pop_id(lower_id);
        assert_eq!(removed.map(|result| result.id), Some(lower_id));
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());

        let closed = modals.pop();
        assert_eq!(closed.map(|result| result.id), Some(upper_id));
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn tab_navigation_trapped_in_modal() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        for id in 1..=5 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        // Push modal with nodes 2 and 3
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);

        // Focus should be on 2
        assert_eq!(modals.focus_manager().current(), Some(2));

        // Tab forward should go to 3
        modals.focus_manager_mut().focus_next();
        assert_eq!(modals.focus_manager().current(), Some(3));

        // Tab forward should wrap to 2 (trapped)
        modals.focus_manager_mut().focus_next();
        assert_eq!(modals.focus_manager().current(), Some(2));

        // Attempt to focus outside trap should fail
        assert!(modals.focus_manager_mut().focus(5).is_none());
        assert_eq!(modals.focus_manager().current(), Some(2));
    }

    #[test]
    fn empty_focus_group_no_panic() {
        let mut modals = FocusAwareModalStack::new();

        // Push modal with empty focus group (edge case).
        // The trap is NOT pushed because the group has no focusable members,
        // preventing a deadlock where no widget could receive focus.
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![]);

        // Should not panic, and focus should NOT be trapped (empty group).
        assert!(!modals.is_focus_trapped());

        // Pop should still work
        modals.pop();
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn rejected_empty_trap_does_not_leave_focus_group_behind() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals.focus_manager_mut().focus(1);
        let group_count_before = modals.focus_manager().group_count();

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![]);

        assert!(!modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().group_count(), group_count_before);
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn late_registered_focus_ids_activate_modal_trap_and_restore_latest_background_selection() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(50));
            graph.insert(make_focus_node(100));
        });

        modals.focus(100);
        let modal_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        assert!(!modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(100));

        modals.focus(50);
        assert_eq!(modals.focus_manager().current(), Some(50));

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
        });
        assert!(modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(1));

        assert!(modals.pop_id(modal_id).is_some());
        assert_eq!(modals.focus_manager().current(), Some(50));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn blurred_pop_all_after_late_trap_activation_restores_background_focus_on_gain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(50));
            graph.insert(make_focus_node(100));
        });

        modals.focus(100);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        modals.focus(50);

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
        });
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        let results = modals.pop_all();
        assert_eq!(results.len(), 1);
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(50));
    }

    #[test]
    fn push_with_trap_does_not_collide_with_existing_group_ids() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(99));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(100));

        let reserved_group_id = FOCUS_GROUP_COUNTER.load(Ordering::Relaxed);
        modals
            .focus_manager_mut()
            .create_group(reserved_group_id, vec![99]);
        modals.focus_manager_mut().focus(100);

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        let _ = modals.pop().unwrap();

        assert!(modals.focus_manager_mut().push_trap(reserved_group_id));
        assert_eq!(modals.focus_manager().current(), Some(99));
    }

    #[test]
    fn pop_id_non_top_modal_rebuilds_focus_traps() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        for id in 1..=6 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        // Initial focus
        modals.focus_manager_mut().focus(1);

        // Push three modals with focus traps.
        let id1 = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![3]);
        let _id3 = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        // Focus should be on node 4 (top modal)
        assert_eq!(modals.focus_manager().current(), Some(4));

        // Pop the BOTTOM modal (id1) by ID - this is non-LIFO.
        modals.pop_id(id1);

        // Focus should still be on the top modal.
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert_eq!(modals.depth(), 2);
        assert!(modals.is_focus_trapped());

        // Pop remaining modals normally. Focus should restore as if the removed modal never
        // existed: top -> next modal -> original background focus.
        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(modals.is_empty());
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_middle_modal_retargets_upper_return_focus() {
        let mut modals = FocusAwareModalStack::new();

        for id in 1..=6 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        modals.focus_manager_mut().focus(1);

        let _id1 = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        let id2 = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![3]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        assert_eq!(modals.focus_manager().current(), Some(4));

        // Remove the middle modal. The top modal should now restore to modal1's focus.
        modals.pop_id(id2);
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert_eq!(modals.depth(), 2);

        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(2));

        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_rebuild_does_not_pollute_focus_history() {
        let mut modals = FocusAwareModalStack::new();

        for id in 1..=6 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        modals.focus_manager_mut().focus(1);
        modals.focus_manager_mut().focus(6);

        let id1 = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![3]);

        modals.pop_id(id1);
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(6));
        assert!(modals.focus_manager_mut().focus_back());
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.focus_manager_mut().focus_back());
    }

    #[test]
    fn pop_id_top_modal_restores_focus_correctly() {
        let mut modals = FocusAwareModalStack::new();

        // Add focusable nodes
        for id in 1..=4 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        // Initial focus
        modals.focus_manager_mut().focus(1);

        // Push two modals
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        let id2 = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![3]);

        assert_eq!(modals.focus_manager().current(), Some(3));

        // Pop the TOP modal by ID - this should work correctly
        modals.pop_id(id2);

        // Focus should restore to modal1's focus (2)
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped()); // Still in modal1's trap

        // Pop the last modal
        modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_top_modal_preserves_underlying_selected_control() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        let upper_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 5]);
        modals.focus(5);
        let _ = modals.focus_manager_mut().take_focus_event();
        let before = modals.focus_manager().focus_change_count();

        assert!(modals.pop_id(upper_id).is_some());
        assert_eq!(modals.focus_manager().current(), Some(3));
        assert_eq!(
            modals.focus_manager_mut().take_focus_event(),
            Some(crate::focus::FocusEvent::FocusMoved { from: 5, to: 3 })
        );
        assert_eq!(modals.focus_manager().focus_change_count(), before + 1);
        assert!(modals.is_focus_trapped());

        let _ = modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn pop_removes_closed_modal_focus_group() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));

        modals.focus_manager_mut().focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);

        let result = modals.pop().unwrap();
        let group_id = result.focus_group_id.unwrap();

        assert!(!modals.focus_manager_mut().push_trap(group_id));
        assert!(!modals.is_focus_trapped());
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn pop_last_modal_clears_invalid_stale_focus_when_no_fallback_exists() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(2));

        modals.focus_manager_mut().focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), Some(2));

        let _ = modals.focus_manager_mut().graph_mut().remove(1);
        let _ = modals.focus_manager_mut().graph_mut().remove(2);

        modals.pop();
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn default_creates_empty_stack() {
        let modals = FocusAwareModalStack::default();
        assert!(modals.is_empty());
        assert_eq!(modals.depth(), 0);
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn with_focus_manager_uses_provided() {
        let mut fm = FocusManager::new();
        fm.graph_mut().insert(make_focus_node(42));
        fm.focus(42);

        let modals = FocusAwareModalStack::with_focus_manager(fm);
        assert!(modals.is_empty());
        assert_eq!(modals.focus_manager().current(), Some(42));
    }

    #[test]
    fn with_focus_manager_rejects_pretrapped_manager() {
        let mut fm = FocusManager::new();
        fm.graph_mut().insert(make_focus_node(1));
        fm.focus(1);
        fm.create_group(7, vec![1]);
        assert!(fm.push_trap(7));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = FocusAwareModalStack::with_focus_manager(fm);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn stack_accessors() {
        let mut modals = FocusAwareModalStack::new();
        assert!(modals.stack().is_empty());
        modals.push(Box::new(WidgetModalEntry::new(StubWidget)));
        assert!(!modals.stack().is_empty());
        assert_eq!(modals.stack_mut().depth(), 1);
    }

    #[test]
    fn with_focus_graph_mut_resyncs_after_panic() {
        let mut modals = FocusAwareModalStack::new();

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1, 2]);
        assert_eq!(modals.focus_manager().current(), Some(1));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            modals.with_focus_graph_mut(|graph| {
                let _ = graph.remove(1);
                panic!("boom");
            });
        }));
        assert!(result.is_err());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn with_focus_graph_mut_repairs_invalid_focus_without_modals() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });
        modals.focus(2);
        assert_eq!(modals.focus_manager().current(), Some(2));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
        });

        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn with_focus_graph_mut_does_not_restore_focus_while_host_blurred() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });
        modals.focus(2);
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
        });

        assert_eq!(modals.focus_manager().current(), None);
    }

    #[test]
    fn focus_call_while_host_blurred_defers_until_focus_gain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(3));
        });
        modals.focus(1);
        let _ = modals.focus_manager_mut().take_focus_event();

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        assert_eq!(modals.focus(3), Some(1));
        assert_eq!(modals.focus_manager().current(), None);
        assert_eq!(
            modals.focus_manager_mut().take_focus_event(),
            Some(crate::focus::FocusEvent::FocusLost { id: 1 })
        );

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(3));
        assert_eq!(
            modals.focus_manager_mut().take_focus_event(),
            Some(crate::focus::FocusEvent::FocusGained { id: 3 })
        );
    }

    #[test]
    fn pop_while_host_blurred_defers_base_focus_restore_until_focus_gain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });
        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), Some(2));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        let result = modals.pop();
        assert!(result.is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn pop_id_last_modal_while_host_blurred_restores_base_focus_on_focus_gain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });
        modals.focus(1);
        let modal_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), Some(2));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        assert!(modals.pop_id(modal_id).is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn pop_id_top_modal_while_host_blurred_restores_underlying_modal_selection_on_focus_gain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });
        modals.focus(1);
        let _lower_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        let upper_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 5]);
        modals.focus(5);
        let _ = modals.focus_manager_mut().take_focus_event();

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        assert!(modals.pop_id(upper_id).is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(3));
    }

    #[test]
    fn pop_all_while_host_blurred_restores_base_focus_on_focus_gain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(3));
        });
        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![3]);
        assert_eq!(modals.focus_manager().current(), Some(3));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        let results = modals.pop_all();
        assert_eq!(results.len(), 2);
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn focus_gain_after_blurred_pop_restores_base_focus_without_intermediate_hop() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(5));
            graph.insert(make_focus_node(10));
        });
        modals.focus(5);
        let _ = modals.focus_manager_mut().take_focus_event();
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![10]);
        let _ = modals.focus_manager_mut().take_focus_event();

        let _ = modals.handle_event(&Event::Focus(false), None);
        let _ = modals.focus_manager_mut().take_focus_event();

        let result = modals.pop();
        assert!(result.is_some());
        assert_eq!(modals.focus_manager().current(), None);

        let before = modals.focus_manager().focus_change_count();
        let _ = modals.handle_event(&Event::Focus(true), None);

        assert_eq!(modals.focus_manager().current(), Some(5));
        assert_eq!(
            modals.focus_manager_mut().take_focus_event(),
            Some(crate::focus::FocusEvent::FocusGained { id: 5 })
        );
        assert_eq!(modals.focus_manager().focus_change_count(), before + 1);
    }

    #[test]
    fn blurred_background_focus_change_after_last_modal_pop_overrides_stale_base_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(3));
        });
        modals.focus(1);
        let _ = modals.focus_manager_mut().take_focus_event();

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        let _ = modals.focus_manager_mut().take_focus_event();
        let _ = modals.handle_event(&Event::Focus(false), None);
        let _ = modals.focus_manager_mut().take_focus_event();

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), None);

        assert_eq!(modals.focus(3), Some(1));
        assert_eq!(modals.focus_manager().current(), None);

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(3));
        assert_eq!(
            modals.focus_manager_mut().take_focus_event(),
            Some(crate::focus::FocusEvent::FocusGained { id: 3 })
        );
    }

    #[test]
    fn pop_id_middle_modal_preserves_top_selection_and_retargets_restore_chain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=7 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        let _lower_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        let middle_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 5]);
        modals.focus(5);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![6, 7]);
        modals.focus(7);
        let _ = modals.focus_manager_mut().take_focus_event();

        let removed = modals.pop_id(middle_id);
        assert!(removed.is_some());
        assert_eq!(modals.focus_manager().current(), Some(7));
        assert!(modals.is_focus_trapped());

        let _ = modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(3));

        let _ = modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_bottom_modal_preserves_top_selection_and_retargets_to_base_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        let lower_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 5]);
        modals.focus(5);
        let _ = modals.focus_manager_mut().take_focus_event();

        let removed = modals.pop_id(lower_id);
        assert!(removed.is_some());
        assert_eq!(modals.focus_manager().current(), Some(5));
        assert!(modals.is_focus_trapped());

        let _ = modals.pop();
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn push_with_trap_while_host_blurred_defers_modal_focus_until_focus_gain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });
        modals.focus(1);
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(2));
    }

    #[test]
    fn nested_push_while_host_blurred_restores_underlying_modal_selection_on_close() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });
        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);
        assert_eq!(modals.focus_manager().current(), None);

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(4));

        let result = modals.pop();
        assert!(result.is_some());
        assert_eq!(modals.focus_manager().current(), Some(3));
    }

    #[test]
    fn first_modal_opened_while_blurred_from_unfocused_base_restores_none() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), None);

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(2));

        let result = modals.pop();
        assert!(result.is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_non_top_while_host_blurred_keeps_focus_cleared_until_focus_gain() {
        let mut modals = FocusAwareModalStack::new();
        for id in 1..=4 {
            modals
                .focus_manager_mut()
                .graph_mut()
                .insert(make_focus_node(id));
        }

        modals.focus_manager_mut().focus(1);
        let id1 = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![3]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);
        assert_eq!(modals.focus_manager().current(), Some(4));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        let result = modals.pop_id(id1);
        assert!(result.is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(4));
    }

    #[test]
    fn pop_id_trapped_modal_while_blurred_with_only_non_trapped_modals_remaining_restores_base_focus()
     {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });

        modals.focus(1);
        let trapped_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push(Box::new(WidgetModalEntry::new(StubWidget)));
        assert_eq!(modals.focus_manager().current(), Some(2));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        assert!(modals.pop_id(trapped_id).is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(1));
    }

    #[test]
    fn pop_id_inactive_trapped_modal_with_only_non_trapped_modals_remaining_preserves_latest_background_focus()
     {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(9));
        });

        modals.focus(1);
        let trapped_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push(Box::new(WidgetModalEntry::new(StubWidget)));
        assert_eq!(modals.focus_manager().current(), Some(2));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
        });
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());

        assert_eq!(modals.focus(9), Some(1));
        assert_eq!(modals.focus_manager().current(), Some(9));
        assert!(!modals.is_focus_trapped());

        assert!(modals.pop_id(trapped_id).is_some());
        assert_eq!(modals.focus_manager().current(), Some(9));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn blurred_pop_id_inactive_trapped_modal_with_only_non_trapped_modals_remaining_preserves_latest_background_focus_on_focus_gain()
     {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(9));
        });

        modals.focus(1);
        let trapped_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push(Box::new(WidgetModalEntry::new(StubWidget)));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
        });
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());

        assert_eq!(modals.focus(9), Some(1));
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        assert!(modals.pop_id(trapped_id).is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(9));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn blurred_pop_id_inactive_trapped_modal_preserves_latest_background_focus_when_trap_went_inactive_while_blurred()
     {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(9));
        });

        modals.focus(1);
        let trapped_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        modals.push(Box::new(WidgetModalEntry::new(StubWidget)));
        assert_eq!(modals.focus_manager().current(), Some(2));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
        });
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        assert_eq!(modals.focus(9), Some(1));
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        assert!(modals.pop_id(trapped_id).is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(9));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn focus_gain_refreshes_inactive_modal_restore_target_after_background_fallback() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(9));
        });

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
        });
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(9));
        assert!(!modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(2));
        });
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(9));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn with_focus_graph_mut_blurred_empty_trap_restores_base_focus_on_focus_gain() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(3));
        });

        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), Some(2));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
        });

        assert_eq!(modals.focus_manager().current(), None);
        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(3));
    }

    #[test]
    fn with_focus_graph_mut_focused_empty_trap_restores_base_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(3));
        });

        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), Some(2));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
        });

        assert_eq!(modals.focus_manager().current(), Some(3));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn with_focus_graph_mut_focused_empty_top_trap_restores_underlying_selected_control() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);
        assert_eq!(modals.focus_manager().current(), Some(4));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });

        assert_eq!(modals.focus_manager().current(), Some(3));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn with_focus_graph_mut_blurred_empty_top_trap_restores_underlying_selected_control_on_focus_gain()
     {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(3));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn with_focus_graph_mut_empty_lower_trap_retargets_surviving_top_restore_to_base_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in [1, 5, 8, 10] {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(10);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![8]);
        assert_eq!(modals.focus_manager().current(), Some(8));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(5);
        });
        assert_eq!(modals.focus_manager().current(), Some(8));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(10));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_after_top_trap_becomes_empty_preserves_underlying_trap() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(3));
        assert!(modals.is_focus_trapped());
        assert_eq!(modals.focus(1), None);
        assert_eq!(modals.focus_manager().current(), Some(3));

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn blurred_pop_after_top_trap_becomes_empty_preserves_underlying_deferred_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(3));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_skips_stale_retarget_from_inactive_middle_modal() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=6 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        let stale_middle_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));
        modals.focus(2);
        assert_eq!(modals.focus_manager().current(), Some(2));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5, 6]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        assert!(modals.pop_id(stale_middle_id).is_some());
        assert_eq!(modals.focus_manager().current(), Some(5));

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn blurred_pop_id_skips_stale_retarget_from_inactive_middle_modal() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=6 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        let stale_middle_id =
            modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        assert_eq!(modals.focus(2), Some(3));
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5, 6]);

        assert!(modals.pop_id(stale_middle_id).is_some());
        assert_eq!(modals.focus_manager().current(), None);

        assert!(modals.pop().is_some());
        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_inactive_lower_modal_preserves_surviving_upper_restore_to_base_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in [5, 10, 20, 30] {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(10);
        let lower_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![20]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![30]);
        assert_eq!(modals.focus_manager().current(), Some(30));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(20);
        });
        assert_eq!(modals.focus_manager().current(), Some(30));

        assert!(modals.pop_id(lower_id).is_some());
        assert_eq!(modals.focus_manager().current(), Some(30));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(10));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn pop_id_active_lower_modal_propagates_none_restore_target_to_surviving_upper_modal() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(1));
            graph.insert(make_focus_node(2));
        });

        let lower_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![1]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2]);
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop_id(lower_id).is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn blurred_pop_id_inactive_lower_modal_preserves_surviving_upper_restore_to_base_focus() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in [5, 10, 20, 30] {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(10);
        let lower_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![20]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![30]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(20);
        });
        assert!(modals.pop_id(lower_id).is_some());

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), None);

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(10));
        assert!(!modals.is_focus_trapped());
    }

    #[test]
    fn reactivated_top_modal_restores_latest_underlying_selection() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.focus(2);
        assert_eq!(modals.focus_manager().current(), Some(2));

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn reactivated_top_modal_tracks_graph_restored_selection_within_same_lower_group() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(3);
        });
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn blurred_reactivated_top_modal_tracks_graph_restored_selection_within_same_lower_group() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(3);
        });
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn blurred_reactivated_top_modal_restores_latest_underlying_selection() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=4 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        assert_eq!(modals.focus(2), Some(3));

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn reactivated_inactive_top_modal_tracks_graph_restored_underlying_selection() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=7 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 5]);
        modals.focus(5);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![6]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![7]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(7);
        });
        assert_eq!(modals.focus_manager().current(), Some(6));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(6);
        });
        assert_eq!(modals.focus_manager().current(), Some(5));
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(7));
        });
        assert_eq!(modals.focus_manager().current(), Some(7));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(5));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn reactivated_inactive_modal_chain_tracks_graph_restored_underlying_selection() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(5);
        });
        assert_eq!(modals.focus_manager().current(), Some(4));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(3);
        });
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(5));
        });
        assert_eq!(modals.focus_manager().current(), Some(5));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn reactivated_lower_modal_refreshes_still_inactive_upper_restore_target() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in [1, 2, 4, 5] {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 4]);
        modals.focus(4);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(5);
        });
        assert_eq!(modals.focus_manager().current(), Some(4));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(2);
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(1));
        assert!(!modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(2));
            graph.insert(make_focus_node(4));
        });
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(5));
        });
        assert_eq!(modals.focus_manager().current(), Some(5));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn reactivated_inactive_upper_modal_does_not_restore_stale_lower_selection_after_top_pop() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(3);
        });
        assert_eq!(modals.focus_manager().current(), Some(5));

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(3));
        });
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn blurred_reactivated_inactive_upper_modal_does_not_restore_stale_lower_selection_after_top_pop()
     {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(3);
        });
        assert_eq!(modals.focus_manager().current(), None);

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(3));
        });
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn reactivated_middle_modal_before_top_close_does_not_restore_stale_lower_selection() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);
        assert_eq!(modals.focus_manager().current(), Some(4));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(3);
        });
        assert_eq!(modals.focus_manager().current(), Some(5));

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        assert_eq!(modals.focus_manager().current(), Some(5));

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn revalidated_stale_lower_target_before_top_close_does_not_win_on_middle_pop() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(3);
        });
        assert_eq!(modals.focus_manager().current(), Some(5));

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        assert_eq!(modals.focus_manager().current(), Some(5));

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(3));
        });
        assert_eq!(modals.focus_manager().current(), Some(5));

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn blurred_revalidated_stale_lower_target_before_top_close_does_not_win_on_middle_pop() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(3));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(3);
        });
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(4));
        });
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(3));
        });
        assert_eq!(modals.focus_manager().current(), None);

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(4));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn invalidated_lower_selection_retargets_upper_restore_using_group_tab_order() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 3, 2]);
        assert_eq!(modals.focus_manager().current(), Some(2));
        modals.focus(4);
        assert_eq!(modals.focus_manager().current(), Some(4));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), Some(5));

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn blurred_invalidated_lower_selection_retargets_upper_restore_using_group_tab_order() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=5 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 3, 2]);
        assert_eq!(modals.focus_manager().current(), Some(2));
        modals.focus(4);
        assert_eq!(modals.focus_manager().current(), Some(4));

        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });
        assert_eq!(modals.focus_manager().current(), None);

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), None);
        assert!(modals.is_focus_trapped());

        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(2));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn invalidated_negative_tabindex_lower_selection_retargets_upper_restore_metadata() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(FocusNode::new(3, Rect::new(0, 0, 10, 3)).with_tab_index(-1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(FocusNode::new(4, Rect::new(0, 0, 10, 3)).with_tab_index(-1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(5));

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 3]);
        assert_eq!(modals.focus_manager().current(), Some(4));

        let upper_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });

        let upper_return_focus = modals
            .stack
            .focus_modal_specs_in_order()
            .into_iter()
            .find(|(modal_id, _)| *modal_id == upper_id)
            .map(|(_, spec)| spec.return_focus);
        assert_eq!(upper_return_focus, Some(Some(3)));
    }

    #[test]
    fn blurred_invalidated_negative_tabindex_lower_selection_retargets_upper_restore_metadata() {
        let mut modals = FocusAwareModalStack::new();
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(FocusNode::new(3, Rect::new(0, 0, 10, 3)).with_tab_index(-1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(FocusNode::new(4, Rect::new(0, 0, 10, 3)).with_tab_index(-1));
        modals
            .focus_manager_mut()
            .graph_mut()
            .insert(make_focus_node(5));

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 3]);
        assert_eq!(modals.focus_manager().current(), Some(4));

        let upper_id = modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![5]);
        assert_eq!(modals.focus_manager().current(), Some(5));

        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(4);
        });

        let upper_return_focus = modals
            .stack
            .focus_modal_specs_in_order()
            .into_iter()
            .find(|(modal_id, _)| *modal_id == upper_id)
            .map(|(_, spec)| spec.return_focus);
        assert_eq!(upper_return_focus, Some(Some(3)));
    }

    #[test]
    fn blurred_reactivated_inactive_top_modal_tracks_graph_restored_underlying_selection() {
        let mut modals = FocusAwareModalStack::new();
        modals.with_focus_graph_mut(|graph| {
            for id in 1..=7 {
                graph.insert(make_focus_node(id));
            }
        });

        modals.focus(1);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![2, 3]);
        modals.focus(3);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![4, 5]);
        modals.focus(5);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![6]);
        modals.push_with_trap(Box::new(WidgetModalEntry::new(StubWidget)), vec![7]);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(7);
        });
        let _ = modals.handle_event(&Event::Focus(false), None);
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            let _ = graph.remove(6);
        });
        assert_eq!(modals.focus_manager().current(), None);

        modals.with_focus_graph_mut(|graph| {
            graph.insert(make_focus_node(7));
        });
        let _ = modals.handle_event(&Event::Focus(true), None);
        assert_eq!(modals.focus_manager().current(), Some(7));
        assert!(modals.is_focus_trapped());

        assert!(modals.pop().is_some());
        assert_eq!(modals.focus_manager().current(), Some(5));
        assert!(modals.is_focus_trapped());
    }

    #[test]
    fn depth_tracks_push_pop() {
        let mut modals = FocusAwareModalStack::new();
        assert_eq!(modals.depth(), 0);
        modals.push(Box::new(WidgetModalEntry::new(StubWidget)));
        assert_eq!(modals.depth(), 1);
        modals.push(Box::new(WidgetModalEntry::new(StubWidget)));
        assert_eq!(modals.depth(), 2);
        modals.pop();
        assert_eq!(modals.depth(), 1);
    }

    #[test]
    fn pop_empty_stack_returns_none() {
        let mut modals = FocusAwareModalStack::new();
        assert!(modals.pop().is_none());
    }
}
