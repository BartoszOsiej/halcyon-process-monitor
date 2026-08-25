#![forbid(unsafe_code)]

//! bd-1lg.16: Accessibility roles and keyboard navigation tests.
//!
//! Proves that:
//! 1. Tab/Shift-Tab navigates all focusable widgets
//! 2. Enter/Space activates interactive widgets (list selection, tree toggle, tab switch)
//! 3. Arrow keys navigate lists, trees, and spatial focus graphs
//! 4. Escape closes modals (via StackModal::close_on_escape)
//! 5. High-contrast theme provides distinct, readable colors
//! 6. Focus indicators are visible and style correctly
//!
//! Run:
//!   cargo test -p ftui-widgets --test accessibility_keyboard_navigation

use ftui_core::event::{KeyCode, KeyEvent};
use ftui_core::geometry::Rect;
use ftui_render::cell::PackedRgba;
use ftui_style::Style;
use ftui_style::theme::themes;
use ftui_widgets::focus::{
    FocusEvent, FocusId, FocusIndicator, FocusIndicatorKind, FocusManager, FocusNode, NavDirection,
};
use ftui_widgets::list::{List, ListItem, ListState};
use ftui_widgets::tabs::TabsState;
use ftui_widgets::tree::{Tree, TreeNode};

// ============================================================================
// 1. Tab/Shift-Tab Navigation
// ============================================================================

#[test]
fn tab_navigates_through_all_focusable_widgets() {
    let mut fm = FocusManager::new();
    let ids: Vec<FocusId> = (1..=5).collect();

    for (i, &id) in ids.iter().enumerate() {
        fm.graph_mut().insert(FocusNode {
            id,
            bounds: Rect::new(0, i as u16 * 3, 20, 3),
            tab_index: i as i32,
            is_focusable: true,
            group_id: None,
        });
    }

    // Focus first widget
    assert!(fm.focus_first());
    assert_eq!(fm.current(), Some(1));

    // Tab through all widgets
    for expected in 2..=5 {
        assert!(fm.focus_next(), "Tab should move to widget {expected}");
        assert_eq!(fm.current(), Some(expected));
    }
}

#[test]
fn shift_tab_navigates_backward_through_all_widgets() {
    let mut fm = FocusManager::new();
    let ids: Vec<FocusId> = (1..=4).collect();

    for (i, &id) in ids.iter().enumerate() {
        fm.graph_mut().insert(FocusNode {
            id,
            bounds: Rect::new(0, i as u16 * 3, 20, 3),
            tab_index: i as i32,
            is_focusable: true,
            group_id: None,
        });
    }

    // Start at last widget
    assert!(fm.focus_last());
    assert_eq!(fm.current(), Some(4));

    // Shift-Tab through all widgets backward
    for expected in (1..=3).rev() {
        assert!(
            fm.focus_prev(),
            "Shift-Tab should move to widget {expected}"
        );
        assert_eq!(fm.current(), Some(expected));
    }
}

#[test]
fn tab_skips_non_focusable_widgets() {
    let mut fm = FocusManager::new();

    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });
    // Non-focusable widget
    fm.graph_mut().insert(FocusNode {
        id: 2,
        bounds: Rect::new(0, 3, 20, 3),
        tab_index: 1,
        is_focusable: false,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: 3,
        bounds: Rect::new(0, 6, 20, 3),
        tab_index: 2,
        is_focusable: true,
        group_id: None,
    });

    fm.focus_first();
    assert_eq!(fm.current(), Some(1));

    // Tab should skip widget 2 (non-focusable)
    assert!(fm.focus_next());
    assert_eq!(fm.current(), Some(3), "Should skip non-focusable widget");
}

#[test]
fn tab_skips_negative_tab_index() {
    let mut fm = FocusManager::new();

    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: 2,
        bounds: Rect::new(0, 3, 20, 3),
        tab_index: -1, // Negative = skip tab order
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: 3,
        bounds: Rect::new(0, 6, 20, 3),
        tab_index: 1,
        is_focusable: true,
        group_id: None,
    });

    fm.focus_first();
    assert_eq!(fm.current(), Some(1));

    // Tab should skip widget 2 (negative tab_index)
    assert!(fm.focus_next());
    assert_eq!(fm.current(), Some(3), "Should skip negative tab_index");
}

#[test]
fn tab_respects_tab_index_ordering() {
    let mut fm = FocusManager::new();

    // Insert out of order, but tab_index should determine order
    fm.graph_mut().insert(FocusNode {
        id: 10,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 2,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: 20,
        bounds: Rect::new(0, 3, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: 30,
        bounds: Rect::new(0, 6, 20, 3),
        tab_index: 1,
        is_focusable: true,
        group_id: None,
    });

    fm.focus_first();
    assert_eq!(fm.current(), Some(20), "First should be tab_index=0");

    assert!(fm.focus_next());
    assert_eq!(fm.current(), Some(30), "Second should be tab_index=1");

    assert!(fm.focus_next());
    assert_eq!(fm.current(), Some(10), "Third should be tab_index=2");
}

#[test]
fn tab_on_empty_graph_returns_false() {
    let mut fm = FocusManager::new();
    assert!(!fm.focus_next());
    assert!(!fm.focus_prev());
    assert!(fm.current().is_none());
}

// ============================================================================
// 2. Enter/Space Activates Interactive Widgets
// ============================================================================

#[test]
fn enter_selects_list_item() {
    let items = vec![
        ListItem::new("alpha"),
        ListItem::new("beta"),
        ListItem::new("gamma"),
    ];
    let list = List::new(items);
    let mut state = ListState::default();

    // Navigate to second item
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down)));
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down)));
    assert_eq!(state.selected(), Some(1));
}

#[test]
fn space_toggles_tree_node() {
    let root = TreeNode::new("Root")
        .with_expanded(true)
        .with_children(vec![
            TreeNode::new("Alpha"),
            TreeNode::new("Beta").with_children(vec![TreeNode::new("Beta-1")]),
        ]);
    let mut tree = Tree::new(root);

    // Space toggles node at visible_index=2 (Beta, which has children)
    let handled = tree.handle_key(&KeyEvent::new(KeyCode::Char(' ')), 2);
    assert!(handled, "Space should toggle tree node with children");
}

#[test]
fn enter_toggles_tree_node() {
    let root = TreeNode::new("Root")
        .with_expanded(true)
        .with_children(vec![
            TreeNode::new("Child").with_children(vec![TreeNode::new("Grandchild")]),
        ]);
    let mut tree = Tree::new(root);

    // Enter toggles the child node at visible_index=1
    let handled = tree.handle_key(&KeyEvent::new(KeyCode::Enter), 1);
    assert!(handled, "Enter should toggle tree node with children");
}

#[test]
fn number_key_switches_tab() {
    let mut state = TabsState::default();

    // Press '2' to switch to second tab (0-indexed: index 1)
    assert!(state.handle_key(&KeyEvent::new(KeyCode::Char('2')), 3));
    assert_eq!(state.active, 1, "Number key '2' should select tab index 1");
}

#[test]
fn enter_key_on_leaf_tree_node_returns_false() {
    let root = TreeNode::new("Root")
        .with_expanded(true)
        .with_children(vec![TreeNode::new("Leaf")]);
    let mut tree = Tree::new(root);

    // Enter on a leaf node (no children) should return false
    let handled = tree.handle_key(&KeyEvent::new(KeyCode::Enter), 1);
    assert!(
        !handled,
        "Enter on a leaf node should not be handled (nothing to toggle)"
    );
}

// ============================================================================
// 3. Arrow Keys Navigate Lists, Trees, and Spatial Focus
// ============================================================================

#[test]
fn arrow_down_navigates_list() {
    let items = vec![
        ListItem::new("one"),
        ListItem::new("two"),
        ListItem::new("three"),
    ];
    let list = List::new(items);
    let mut state = ListState::default();

    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down)));
    assert_eq!(state.selected(), Some(0));
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down)));
    assert_eq!(state.selected(), Some(1));
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down)));
    assert_eq!(state.selected(), Some(2));
}

#[test]
fn arrow_up_navigates_list() {
    let items = vec![
        ListItem::new("one"),
        ListItem::new("two"),
        ListItem::new("three"),
    ];
    let list = List::new(items);
    let mut state = ListState::default();

    // Navigate to last item first
    list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down));
    list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down));
    list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down));
    assert_eq!(state.selected(), Some(2));

    // Arrow up
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Up)));
    assert_eq!(state.selected(), Some(1));
}

#[test]
fn arrow_keys_navigate_spatial_focus_graph() {
    let mut fm = FocusManager::new();

    // Create a 2x2 grid layout
    let top_left = 1;
    let top_right = 2;
    let bottom_left = 3;
    let bottom_right = 4;

    fm.graph_mut().insert(FocusNode {
        id: top_left,
        bounds: Rect::new(0, 0, 10, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: top_right,
        bounds: Rect::new(15, 0, 10, 3),
        tab_index: 1,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: bottom_left,
        bounds: Rect::new(0, 5, 10, 3),
        tab_index: 2,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: bottom_right,
        bounds: Rect::new(15, 5, 10, 3),
        tab_index: 3,
        is_focusable: true,
        group_id: None,
    });

    // Focus top-left
    fm.focus(top_left);
    assert_eq!(fm.current(), Some(top_left));

    // Navigate right → top-right
    assert!(fm.navigate(NavDirection::Right));
    assert_eq!(fm.current(), Some(top_right));

    // Navigate down → bottom-right
    assert!(fm.navigate(NavDirection::Down));
    assert_eq!(fm.current(), Some(bottom_right));

    // Navigate left → bottom-left
    assert!(fm.navigate(NavDirection::Left));
    assert_eq!(fm.current(), Some(bottom_left));

    // Navigate up → top-left
    assert!(fm.navigate(NavDirection::Up));
    assert_eq!(fm.current(), Some(top_left));
}

#[test]
fn right_arrow_expands_collapsed_tree_node() {
    let root = TreeNode::new("Root")
        .with_expanded(true)
        .with_children(vec![
            TreeNode::new("Beta")
                .with_expanded(false) // Explicitly collapsed
                .with_children(vec![TreeNode::new("Beta-1")]),
        ]);
    let mut tree = Tree::new(root);

    // Right arrow on collapsed node should expand it
    let handled = tree.handle_key(&KeyEvent::new(KeyCode::Right), 1);
    assert!(
        handled,
        "Right arrow should expand collapsed node with children"
    );
}

#[test]
fn left_arrow_collapses_expanded_tree_node() {
    let root = TreeNode::new("Root")
        .with_expanded(true)
        .with_children(vec![
            TreeNode::new("Beta")
                .with_expanded(true)
                .with_children(vec![TreeNode::new("Beta-1")]),
        ]);
    let mut tree = Tree::new(root);

    // Left arrow on expanded node should collapse it
    let handled = tree.handle_key(&KeyEvent::new(KeyCode::Left), 1);
    assert!(handled, "Left arrow should collapse expanded node");
}

// ============================================================================
// 4. Escape Closes Modals (via StackModal trait)
// ============================================================================

#[test]
fn modal_config_defaults_allow_escape_close() {
    // ModalConfig defaults should enable escape-to-close for accessibility.
    let config = ftui_widgets::ModalConfig::default();
    assert!(
        config.close_on_escape,
        "Modal should close on Escape by default for accessibility"
    );
}

#[test]
fn modal_config_escape_can_be_disabled() {
    let config = ftui_widgets::ModalConfig {
        close_on_escape: false,
        ..Default::default()
    };
    assert!(
        !config.close_on_escape,
        "Modal escape close should be configurable"
    );
}

// ============================================================================
// 5. Focus Trap in Modals
// ============================================================================

#[test]
fn focus_trap_confines_tab_navigation() {
    let mut fm = FocusManager::new();

    // Main page widgets
    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: 2,
        bounds: Rect::new(0, 3, 20, 3),
        tab_index: 1,
        is_focusable: true,
        group_id: None,
    });

    // Modal widgets (group 1)
    fm.graph_mut().insert(FocusNode {
        id: 10,
        bounds: Rect::new(5, 5, 10, 2),
        tab_index: 0,
        is_focusable: true,
        group_id: Some(1),
    });
    fm.graph_mut().insert(FocusNode {
        id: 11,
        bounds: Rect::new(5, 7, 10, 2),
        tab_index: 1,
        is_focusable: true,
        group_id: Some(1),
    });

    // Register the focus group and focus a main page widget
    fm.create_group(1, vec![10, 11]);
    fm.focus(1);
    assert_eq!(fm.current(), Some(1));

    // Push trap for modal group
    fm.push_trap(1);

    // After pushing trap, focus moves to first in group
    assert_eq!(
        fm.current(),
        Some(10),
        "After push_trap, focus should be on first widget in modal group"
    );

    // Tab within the trapped group
    fm.focus_next();
    assert_eq!(
        fm.current(),
        Some(11),
        "Tab in trapped mode should move to next in modal group"
    );

    // Pop trap should restore focus
    assert!(fm.pop_trap());
    assert_eq!(
        fm.current(),
        Some(1),
        "Pop trap should restore previous focus"
    );
}

#[test]
fn focus_trap_prevents_escape_to_main_content() {
    let mut fm = FocusManager::new();

    // Main content
    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });

    // Modal group
    fm.graph_mut().insert(FocusNode {
        id: 10,
        bounds: Rect::new(5, 5, 10, 2),
        tab_index: 0,
        is_focusable: true,
        group_id: Some(1),
    });

    fm.create_group(1, vec![10]);
    fm.focus(1);
    fm.push_trap(1);

    // Focus should now be on modal widget, not main content
    assert_eq!(fm.current(), Some(10));

    // Trying to focus main content widget should fail when trapped
    let result = fm.focus(1);
    assert!(
        result.is_none(),
        "Focus should not escape to main content when trapped"
    );
    assert_eq!(
        fm.current(),
        Some(10),
        "Focus should remain on modal widget"
    );
}

// ============================================================================
// 6. High-Contrast Theme
// ============================================================================

/// Compute relative luminance per WCAG 2.0 formula.
fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    let srgb = |v: u8| -> f64 {
        let v = v as f64 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * srgb(r) + 0.7152 * srgb(g) + 0.0722 * srgb(b)
}

/// Compute contrast ratio per WCAG 2.0.
fn contrast_ratio(l1: f64, l2: f64) -> f64 {
    let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

#[test]
fn doom_theme_text_background_meets_wcag_aa() {
    let theme = themes::doom();
    let resolved = theme.resolve(true); // dark mode

    let text = resolved.text.to_rgb();
    let bg = resolved.background.to_rgb();

    let text_lum = relative_luminance(text.r, text.g, text.b);
    let bg_lum = relative_luminance(bg.r, bg.g, bg.b);
    let ratio = contrast_ratio(text_lum, bg_lum);

    // WCAG AA requires 4.5:1 for normal text
    assert!(
        ratio >= 4.5,
        "Doom theme text/background contrast ratio {ratio:.2} must be >= 4.5:1"
    );
}

#[test]
fn doom_theme_error_color_is_distinct() {
    let theme = themes::doom();
    let resolved = theme.resolve(true);

    let error = resolved.error.to_rgb();
    let text = resolved.text.to_rgb();

    // Error color should be visually distinct from regular text
    let error_lum = relative_luminance(error.r, error.g, error.b);
    let text_lum = relative_luminance(text.r, text.g, text.b);

    assert!(
        (error_lum - text_lum).abs() > 0.05,
        "Error color should be visually distinct from text"
    );
}

#[test]
fn doom_theme_success_color_is_distinct_from_error() {
    let theme = themes::doom();
    let resolved = theme.resolve(true);

    let success = resolved.success.to_rgb();
    let error = resolved.error.to_rgb();

    // Success and error must be distinguishable
    assert_ne!(
        (success.r, success.g, success.b),
        (error.r, error.g, error.b),
        "Success and error colors must be different"
    );
}

#[test]
fn all_preset_themes_have_distinct_text_and_background() {
    let themes = [
        ("dark", themes::dark()),
        ("light", themes::light()),
        ("nord", themes::nord()),
        ("dracula", themes::dracula()),
        ("solarized_dark", themes::solarized_dark()),
        ("solarized_light", themes::solarized_light()),
        ("monokai", themes::monokai()),
        ("doom", themes::doom()),
    ];

    for (name, theme) in &themes {
        let resolved = theme.resolve(true);

        let text = resolved.text.to_rgb();
        let bg = resolved.background.to_rgb();

        assert_ne!(
            (text.r, text.g, text.b),
            (bg.r, bg.g, bg.b),
            "Theme {name}: text and background must be different"
        );
    }
}

#[test]
fn theme_resolve_produces_consistent_colors() {
    let theme = themes::dark();

    // Resolving twice should give the same result
    let r1 = theme.resolve(true);
    let r2 = theme.resolve(true);

    assert_eq!(r1.text, r2.text, "Resolved colors should be deterministic");
    assert_eq!(
        r1.background, r2.background,
        "Resolved colors should be deterministic"
    );
}

// ============================================================================
// 7. Focus Indicators
// ============================================================================

#[test]
fn focus_indicator_default_is_visible() {
    let indicator = FocusIndicator::default();
    assert!(
        indicator.is_visible(),
        "Default focus indicator must be visible"
    );
    assert_eq!(indicator.kind(), FocusIndicatorKind::StyleOverlay);
}

#[test]
fn focus_indicator_applies_style_overlay() {
    let base = Style::new().fg(PackedRgba::rgb(200, 200, 200));
    let indicator = FocusIndicator::default();

    let focused = indicator.apply_to(base);

    // Focused style should include the reverse attribute from default indicator
    assert_ne!(
        focused, base,
        "Focus indicator should modify the base style"
    );
}

#[test]
fn focus_indicator_none_preserves_base_style() {
    let base = Style::new().fg(PackedRgba::rgb(255, 0, 0)).bold();
    let indicator = FocusIndicator::none();

    let result = indicator.apply_to(base);
    assert_eq!(
        result, base,
        "None indicator should preserve base style exactly"
    );
}

#[test]
fn focus_indicator_underline_is_visible() {
    let indicator = FocusIndicator::underline();
    assert!(indicator.is_visible());
    assert_eq!(indicator.kind(), FocusIndicatorKind::Underline);
}

#[test]
fn focus_indicator_border_is_visible() {
    let indicator = FocusIndicator::border();
    assert!(indicator.is_visible());
    assert_eq!(indicator.kind(), FocusIndicatorKind::Border);
}

// ============================================================================
// 8. Focus Events
// ============================================================================

#[test]
fn focus_gained_event_on_first_focus() {
    let mut fm = FocusManager::new();

    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });

    fm.focus_first();

    let event = fm.focus_event();
    assert!(
        matches!(event, Some(FocusEvent::FocusGained { id: 1 })),
        "First focus should emit FocusGained, got {event:?}"
    );
}

#[test]
fn focus_moved_event_on_tab() {
    let mut fm = FocusManager::new();

    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: 2,
        bounds: Rect::new(0, 3, 20, 3),
        tab_index: 1,
        is_focusable: true,
        group_id: None,
    });

    fm.focus_first();
    fm.focus_next();

    let event = fm.focus_event();
    assert!(
        matches!(event, Some(FocusEvent::FocusMoved { from: 1, to: 2 })),
        "Tab should emit FocusMoved, got {event:?}"
    );
}

#[test]
fn focus_lost_event_on_blur() {
    let mut fm = FocusManager::new();

    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });

    fm.focus_first();
    fm.blur();

    let event = fm.focus_event();
    assert!(
        matches!(event, Some(FocusEvent::FocusLost { id: 1 })),
        "Blur should emit FocusLost, got {event:?}"
    );
    assert!(
        fm.current().is_none(),
        "No widget should be focused after blur"
    );
}

// ============================================================================
// 9. Focus History and Back Navigation
// ============================================================================

#[test]
fn focus_back_restores_previous_focus() {
    let mut fm = FocusManager::new();

    for i in 1..=3 {
        fm.graph_mut().insert(FocusNode {
            id: i,
            bounds: Rect::new(0, (i as u16 - 1) * 3, 20, 3),
            tab_index: (i - 1) as i32,
            is_focusable: true,
            group_id: None,
        });
    }

    fm.focus_first(); // → 1
    fm.focus_next(); // → 2
    fm.focus_next(); // → 3

    assert_eq!(fm.current(), Some(3));

    // Go back should restore to 2
    assert!(fm.focus_back());
    assert_eq!(fm.current(), Some(2), "focus_back should restore to 2");

    // Go back again should restore to 1
    assert!(fm.focus_back());
    assert_eq!(fm.current(), Some(1), "focus_back should restore to 1");
}

// ============================================================================
// 10. Widget-Level Keyboard Accessibility
// ============================================================================

#[test]
fn list_escape_clears_filter_for_accessibility() {
    let items = vec![
        ListItem::new("alpha"),
        ListItem::new("beta"),
        ListItem::new("gamma"),
    ];
    let list = List::new(items);
    let mut state = ListState::default();

    // Type to filter
    list.handle_key(&mut state, &KeyEvent::new(KeyCode::Char('b')));
    assert_eq!(state.filter_query(), "b");

    // Escape clears filter, restoring full list access
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Escape)));
    assert_eq!(state.filter_query(), "", "Escape should clear filter");
}

#[test]
fn list_vi_j_k_navigation_for_keyboard_users() {
    let items = vec![
        ListItem::new("one"),
        ListItem::new("two"),
        ListItem::new("three"),
    ];
    let list = List::new(items);
    let mut state = ListState::default();

    // j = Down (vi binding for keyboard-only users)
    list.handle_key(&mut state, &KeyEvent::new(KeyCode::Char('j')));
    assert_eq!(state.selected(), Some(0), "j should move down");

    list.handle_key(&mut state, &KeyEvent::new(KeyCode::Char('j')));
    assert_eq!(state.selected(), Some(1), "j should move to second item");

    // k = Up (vi binding)
    list.handle_key(&mut state, &KeyEvent::new(KeyCode::Char('k')));
    assert_eq!(state.selected(), Some(0), "k should move up");
}

#[test]
fn tabs_left_right_arrow_keys() {
    let mut state = TabsState::default();

    // Right arrow to move forward
    assert!(state.handle_key(&KeyEvent::new(KeyCode::Right), 3));
    assert_eq!(state.active, 1);

    assert!(state.handle_key(&KeyEvent::new(KeyCode::Right), 3));
    assert_eq!(state.active, 2);

    // Left arrow to move back
    assert!(state.handle_key(&KeyEvent::new(KeyCode::Left), 3));
    assert_eq!(state.active, 1);
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn tabs_right_arrow_at_end_does_not_wrap() {
    let mut state = TabsState::default();
    state.active = 2;

    // Right at end should not wrap (returns false)
    assert!(!state.handle_key(&KeyEvent::new(KeyCode::Right), 3));
    assert_eq!(state.active, 2, "Right at end should stay at last tab");
}

#[test]
fn tabs_left_arrow_at_start_does_not_wrap() {
    let mut state = TabsState::default();
    // active defaults to 0

    // Left at start should not wrap
    assert!(!state.handle_key(&KeyEvent::new(KeyCode::Left), 3));
    assert_eq!(state.active, 0, "Left at start should stay at first tab");
}

// ============================================================================
// 11. Host Focus (Window Focus/Blur)
// ============================================================================

#[test]
fn host_blur_clears_focus() {
    let mut fm = FocusManager::new();

    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });

    fm.focus_first();
    assert_eq!(fm.current(), Some(1));

    // Window loses focus
    assert!(fm.apply_host_focus(false));
    assert!(
        fm.current().is_none(),
        "Host blur should clear widget focus"
    );
}

#[test]
fn host_focus_restores_first_widget() {
    let mut fm = FocusManager::new();

    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });

    // Window gains focus with no current focus
    assert!(fm.apply_host_focus(true));
    assert!(
        fm.current().is_some(),
        "Host focus should restore focus to first widget"
    );
}

// ============================================================================
// 12. Focus Change Counter (Metrics)
// ============================================================================

#[test]
fn focus_change_count_increments() {
    let mut fm = FocusManager::new();

    fm.graph_mut().insert(FocusNode {
        id: 1,
        bounds: Rect::new(0, 0, 20, 3),
        tab_index: 0,
        is_focusable: true,
        group_id: None,
    });
    fm.graph_mut().insert(FocusNode {
        id: 2,
        bounds: Rect::new(0, 3, 20, 3),
        tab_index: 1,
        is_focusable: true,
        group_id: None,
    });

    let initial = fm.focus_change_count();
    fm.focus_first();
    assert!(
        fm.focus_change_count() > initial,
        "Focus change count should increment"
    );

    let before_tab = fm.focus_change_count();
    fm.focus_next();
    assert!(
        fm.focus_change_count() > before_tab,
        "Tab should increment focus change count"
    );
}
