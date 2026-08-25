#![forbid(unsafe_code)]

//! bd-3rrzt.4: Integration and E2E tests for session-type terminal modes.
//!
//! Validates:
//! 1. Every valid typestate transition produces correct flag state.
//! 2. Teardown from any reachable state returns to cooked (flags == 0).
//! 3. Builder matches typestate flags for equivalent sequences.
//! 4. Re-entry after teardown starts from clean state.
//! 5. Full lifecycle: cooked → raw → alt → features → teardown → re-enter.
//! 6. Composite state enable/disable round-trips.
//!
//! All tests emit structured JSONL for deterministic replay.
//!
//! Run:
//!   cargo test -p ftui-core --test mode_typestate_e2e

use ftui_core::mode_typestate::*;

// ============================================================================
// JSONL Log Entry
// ============================================================================

#[derive(serde::Serialize)]
struct TransitionLog {
    test: String,
    transition: String,
    from_flags: u8,
    to_flags: u8,
    expected_flags: u8,
    correct: bool,
}

#[derive(serde::Serialize)]
struct LifecycleLog {
    test: String,
    step: usize,
    action: String,
    flags: u8,
    modes_active: Vec<String>,
}

fn describe_flags(flags: u8) -> Vec<String> {
    let mut modes = Vec::new();
    if flags & RAW != 0 {
        modes.push("Raw".to_string());
    }
    if flags & ALT_SCREEN != 0 {
        modes.push("AltScreen".to_string());
    }
    if flags & MOUSE != 0 {
        modes.push("Mouse".to_string());
    }
    if flags & BRACKETED_PASTE != 0 {
        modes.push("BracketedPaste".to_string());
    }
    if flags & FOCUS_EVENTS != 0 {
        modes.push("FocusEvents".to_string());
    }
    if modes.is_empty() {
        modes.push("Cooked".to_string());
    }
    modes
}

// ============================================================================
// 1. Transition Verification
// ============================================================================

#[test]
fn integration_all_forward_transitions() {
    let mut logs = Vec::new();

    // Cooked → Raw
    let cooked = TerminalMode::<COOKED>::new();
    let raw = cooked.enter_raw();
    logs.push(TransitionLog {
        test: "forward_transitions".to_string(),
        transition: "Cooked→Raw".to_string(),
        from_flags: COOKED,
        to_flags: raw.flags(),
        expected_flags: RAW,
        correct: raw.flags() == RAW,
    });
    assert_eq!(raw.flags(), RAW);

    // Raw → AltScreen
    let alt = raw.enter_alt_screen();
    logs.push(TransitionLog {
        test: "forward_transitions".to_string(),
        transition: "Raw→AltScreen".to_string(),
        from_flags: RAW,
        to_flags: alt.flags(),
        expected_flags: RAW | ALT_SCREEN,
        correct: alt.flags() == RAW | ALT_SCREEN,
    });
    assert_eq!(alt.flags(), RAW | ALT_SCREEN);

    // AltScreen → +Mouse
    let with_mouse = alt.enable_mouse();
    logs.push(TransitionLog {
        test: "forward_transitions".to_string(),
        transition: "AltScreen→+Mouse".to_string(),
        from_flags: RAW | ALT_SCREEN,
        to_flags: with_mouse.flags(),
        expected_flags: RAW | ALT_SCREEN | MOUSE,
        correct: with_mouse.flags() == RAW | ALT_SCREEN | MOUSE,
    });
    assert_eq!(with_mouse.flags(), RAW | ALT_SCREEN | MOUSE);

    // +Mouse → +BracketedPaste
    let with_paste = with_mouse.enable_bracketed_paste();
    logs.push(TransitionLog {
        test: "forward_transitions".to_string(),
        transition: "+Mouse→+BracketedPaste".to_string(),
        from_flags: RAW | ALT_SCREEN | MOUSE,
        to_flags: with_paste.flags(),
        expected_flags: RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE,
        correct: with_paste.flags() == RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE,
    });
    assert_eq!(
        with_paste.flags(),
        RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE
    );

    // +BracketedPaste → +FocusEvents (= TUI_FULL)
    let full = with_paste.enable_focus_events();
    logs.push(TransitionLog {
        test: "forward_transitions".to_string(),
        transition: "+BracketedPaste→+FocusEvents".to_string(),
        from_flags: RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE,
        to_flags: full.flags(),
        expected_flags: TUI_FULL,
        correct: full.flags() == TUI_FULL,
    });
    assert_eq!(full.flags(), TUI_FULL);

    // Verify JSONL compliance.
    for log in &logs {
        let json = serde_json::to_string(log).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["correct"].as_bool().unwrap());
    }
    assert!(logs.iter().all(|l| l.correct));

    eprintln!(
        "--- forward_transitions: {} transitions, all correct ---",
        logs.len()
    );
}

#[test]
fn integration_backward_transitions() {
    let mut logs = Vec::new();

    // Build up to composite state.
    let alt = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_mouse()
        .enable_bracketed_paste();

    // Disable bracketed paste.
    let no_paste = alt.disable_bracketed_paste();
    logs.push(TransitionLog {
        test: "backward_transitions".to_string(),
        transition: "-BracketedPaste".to_string(),
        from_flags: RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE,
        to_flags: no_paste.flags(),
        expected_flags: RAW | ALT_SCREEN | MOUSE,
        correct: no_paste.flags() == RAW | ALT_SCREEN | MOUSE,
    });
    assert_eq!(no_paste.flags(), RAW | ALT_SCREEN | MOUSE);

    // Disable mouse.
    let no_mouse = no_paste.disable_mouse();
    logs.push(TransitionLog {
        test: "backward_transitions".to_string(),
        transition: "-Mouse".to_string(),
        from_flags: RAW | ALT_SCREEN | MOUSE,
        to_flags: no_mouse.flags(),
        expected_flags: RAW | ALT_SCREEN,
        correct: no_mouse.flags() == RAW | ALT_SCREEN,
    });
    assert_eq!(no_mouse.flags(), RAW | ALT_SCREEN);

    // Exit alt screen.
    let raw = no_mouse.exit_alt_screen();
    logs.push(TransitionLog {
        test: "backward_transitions".to_string(),
        transition: "ExitAltScreen".to_string(),
        from_flags: RAW | ALT_SCREEN,
        to_flags: raw.flags(),
        expected_flags: RAW,
        correct: raw.flags() == RAW,
    });
    assert_eq!(raw.flags(), RAW);

    // Exit raw.
    let cooked = raw.exit_raw();
    logs.push(TransitionLog {
        test: "backward_transitions".to_string(),
        transition: "ExitRaw".to_string(),
        from_flags: RAW,
        to_flags: cooked.flags(),
        expected_flags: COOKED,
        correct: cooked.flags() == COOKED,
    });
    assert_eq!(cooked.flags(), COOKED);

    assert!(logs.iter().all(|l| l.correct));
    eprintln!(
        "--- backward_transitions: {} transitions, all correct ---",
        logs.len()
    );
}

// ============================================================================
// 2. Teardown From All Reachable States
// ============================================================================

#[test]
fn integration_teardown_from_alt_screen() {
    let alt = TerminalMode::<COOKED>::new().enter_raw().enter_alt_screen();
    let cooked = alt.teardown();
    assert_eq!(cooked.flags(), COOKED);
}

#[test]
fn integration_teardown_from_mouse() {
    let mode = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_mouse();
    let cooked = mode.teardown();
    assert_eq!(cooked.flags(), COOKED);
}

#[test]
fn integration_teardown_from_paste() {
    let mode = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_bracketed_paste();
    let cooked = mode.teardown();
    assert_eq!(cooked.flags(), COOKED);
}

#[test]
fn integration_teardown_from_mouse_paste() {
    let mode = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_mouse()
        .enable_bracketed_paste();
    let cooked = mode.teardown();
    assert_eq!(cooked.flags(), COOKED);
}

#[test]
fn integration_teardown_from_full() {
    let mode = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_mouse()
        .enable_bracketed_paste()
        .enable_focus_events();
    assert_eq!(mode.flags(), TUI_FULL);
    let cooked = mode.teardown();
    assert_eq!(cooked.flags(), COOKED);
}

// ============================================================================
// 3. Builder Matches Typestate Flags
// ============================================================================

#[test]
fn integration_builder_matches_typestate() {
    // Build via typestate.
    let typestate_flags = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_mouse()
        .flags();

    // Build via builder.
    let builder_flags = ModeBuilder::new().raw().alt_screen().mouse().flags();

    assert_eq!(typestate_flags, builder_flags);

    // Full TUI.
    let typestate_full = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_mouse()
        .enable_bracketed_paste()
        .enable_focus_events()
        .flags();
    let builder_full = ModeBuilder::new()
        .raw()
        .alt_screen()
        .mouse()
        .bracketed_paste()
        .focus_events()
        .flags();

    assert_eq!(typestate_full, builder_full);
    assert_eq!(typestate_full, TUI_FULL);
}

// ============================================================================
// 4. Re-Entry After Teardown
// ============================================================================

#[test]
fn integration_reentry_after_teardown() {
    // First session: enter full TUI, tear down.
    let first = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_mouse();
    assert_eq!(first.flags(), RAW | ALT_SCREEN | MOUSE);

    let cooked = first.teardown();
    assert_eq!(cooked.flags(), COOKED);

    // Second session: re-enter from clean state.
    let second = cooked.enter_raw().enter_alt_screen();
    assert_eq!(second.flags(), RAW | ALT_SCREEN);
    assert!(!second.has_mouse()); // Mouse not enabled in second session.
}

// ============================================================================
// 5. Full E2E Lifecycle With JSONL
// ============================================================================

#[test]
fn e2e_full_lifecycle_with_logging() {
    let mut logs = Vec::new();
    let mut step = 0usize;

    // Step 0: Create cooked terminal.
    let cooked = TerminalMode::<COOKED>::new();
    logs.push(LifecycleLog {
        test: "full_lifecycle".to_string(),
        step,
        action: "create_cooked".to_string(),
        flags: cooked.flags(),
        modes_active: describe_flags(cooked.flags()),
    });
    step += 1;
    assert_eq!(cooked.flags(), COOKED);

    // Step 1: Enter raw.
    let raw = cooked.enter_raw();
    logs.push(LifecycleLog {
        test: "full_lifecycle".to_string(),
        step,
        action: "enter_raw".to_string(),
        flags: raw.flags(),
        modes_active: describe_flags(raw.flags()),
    });
    step += 1;
    assert!(raw.has_raw());

    // Step 2: Enter alt screen.
    let alt = raw.enter_alt_screen();
    logs.push(LifecycleLog {
        test: "full_lifecycle".to_string(),
        step,
        action: "enter_alt_screen".to_string(),
        flags: alt.flags(),
        modes_active: describe_flags(alt.flags()),
    });
    step += 1;
    assert!(alt.has_alt_screen());

    // Step 3: Enable mouse.
    let with_mouse = alt.enable_mouse();
    logs.push(LifecycleLog {
        test: "full_lifecycle".to_string(),
        step,
        action: "enable_mouse".to_string(),
        flags: with_mouse.flags(),
        modes_active: describe_flags(with_mouse.flags()),
    });
    step += 1;
    assert!(with_mouse.has_mouse());

    // Step 4: Enable bracketed paste.
    let with_paste = with_mouse.enable_bracketed_paste();
    logs.push(LifecycleLog {
        test: "full_lifecycle".to_string(),
        step,
        action: "enable_bracketed_paste".to_string(),
        flags: with_paste.flags(),
        modes_active: describe_flags(with_paste.flags()),
    });
    step += 1;
    assert!(with_paste.has_bracketed_paste());

    // Step 5: Enable focus events → TUI_FULL.
    let full = with_paste.enable_focus_events();
    logs.push(LifecycleLog {
        test: "full_lifecycle".to_string(),
        step,
        action: "enable_focus_events".to_string(),
        flags: full.flags(),
        modes_active: describe_flags(full.flags()),
    });
    step += 1;
    assert_eq!(full.flags(), TUI_FULL);
    assert!(full.has_focus_events());

    // Step 6: Teardown → Cooked.
    let cooked_again = full.teardown();
    logs.push(LifecycleLog {
        test: "full_lifecycle".to_string(),
        step,
        action: "teardown".to_string(),
        flags: cooked_again.flags(),
        modes_active: describe_flags(cooked_again.flags()),
    });
    step += 1;
    assert_eq!(cooked_again.flags(), COOKED);

    // Step 7: Re-enter for second session.
    let second = cooked_again.enter_raw().enter_alt_screen();
    logs.push(LifecycleLog {
        test: "full_lifecycle".to_string(),
        step,
        action: "reenter_raw_alt".to_string(),
        flags: second.flags(),
        modes_active: describe_flags(second.flags()),
    });
    assert_eq!(second.flags(), RAW | ALT_SCREEN);

    // Verify JSONL compliance.
    for log in &logs {
        let json = serde_json::to_string(log).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["step"].is_u64());
        assert!(parsed["action"].is_string());
        assert!(parsed["flags"].is_u64());
        assert!(parsed["modes_active"].is_array());
    }

    eprintln!("--- e2e_full_lifecycle ---");
    for log in &logs {
        eprintln!(
            "  step={} action={:25} flags={:#04x} modes={:?}",
            log.step, log.action, log.flags, log.modes_active
        );
    }
}

// ============================================================================
// 6. Composite Enable/Disable Round-Trips
// ============================================================================

#[test]
fn integration_mouse_enable_disable_roundtrip() {
    let base = TerminalMode::<COOKED>::new().enter_raw().enter_alt_screen();
    let with = base.enable_mouse();
    assert!(with.has_mouse());
    let without = with.disable_mouse();
    assert!(!without.has_mouse());
    assert_eq!(without.flags(), RAW | ALT_SCREEN);
}

#[test]
fn integration_paste_enable_disable_roundtrip() {
    let base = TerminalMode::<COOKED>::new().enter_raw().enter_alt_screen();
    let with = base.enable_bracketed_paste();
    assert!(with.has_bracketed_paste());
    let without = with.disable_bracketed_paste();
    assert!(!without.has_bracketed_paste());
    assert_eq!(without.flags(), RAW | ALT_SCREEN);
}

#[test]
fn integration_mouse_paste_independent_disable() {
    let both = TerminalMode::<COOKED>::new()
        .enter_raw()
        .enter_alt_screen()
        .enable_mouse()
        .enable_bracketed_paste();
    assert!(both.has_mouse());
    assert!(both.has_bracketed_paste());

    // Disable mouse keeps paste.
    let paste_only = both.disable_mouse();
    assert!(!paste_only.has_mouse());
    assert!(paste_only.has_bracketed_paste());

    // Re-enable mouse from paste-only state.
    let both_again = paste_only.enable_mouse();
    assert!(both_again.has_mouse());
    assert!(both_again.has_bracketed_paste());

    // Disable paste keeps mouse.
    let mouse_only = both_again.disable_bracketed_paste();
    assert!(mouse_only.has_mouse());
    assert!(!mouse_only.has_bracketed_paste());
}

// ============================================================================
// 7. Debug Format Verification
// ============================================================================

#[test]
fn integration_debug_format_all_states() {
    let mut logs = Vec::new();

    let cooked = TerminalMode::<COOKED>::new();
    let debug = format!("{cooked:?}");
    assert_eq!(debug, "TerminalMode<Cooked>");
    logs.push(("Cooked", debug));

    let raw = cooked.enter_raw();
    let debug = format!("{raw:?}");
    assert!(debug.contains("Raw"));
    assert!(!debug.contains("AltScreen"));
    logs.push(("Raw", debug));

    let alt = raw.enter_alt_screen();
    let debug = format!("{alt:?}");
    assert!(debug.contains("Raw"));
    assert!(debug.contains("AltScreen"));
    logs.push(("Raw+AltScreen", debug));

    let full = alt
        .enable_mouse()
        .enable_bracketed_paste()
        .enable_focus_events();
    let debug = format!("{full:?}");
    assert!(debug.contains("Raw"));
    assert!(debug.contains("AltScreen"));
    assert!(debug.contains("Mouse"));
    assert!(debug.contains("BracketedPaste"));
    assert!(debug.contains("FocusEvents"));
    logs.push(("TUI_FULL", debug));

    eprintln!("--- debug_format ---");
    for (name, dbg) in &logs {
        eprintln!("  {name}: {dbg}");
    }
}

// ============================================================================
// 8. JSONL Schema Compliance
// ============================================================================

#[test]
fn e2e_jsonl_schema_compliance() {
    // Transition log.
    let tlog = TransitionLog {
        test: "schema_test".to_string(),
        transition: "Cooked→Raw".to_string(),
        from_flags: COOKED,
        to_flags: RAW,
        expected_flags: RAW,
        correct: true,
    };
    let json = serde_json::to_string(&tlog).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["test"].is_string());
    assert!(parsed["transition"].is_string());
    assert!(parsed["from_flags"].is_u64());
    assert!(parsed["to_flags"].is_u64());
    assert!(parsed["expected_flags"].is_u64());
    assert!(parsed["correct"].is_boolean());

    // Lifecycle log.
    let llog = LifecycleLog {
        test: "schema_test".to_string(),
        step: 0,
        action: "create_cooked".to_string(),
        flags: COOKED,
        modes_active: describe_flags(COOKED),
    };
    let json = serde_json::to_string(&llog).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["test"].is_string());
    assert!(parsed["step"].is_u64());
    assert!(parsed["action"].is_string());
    assert!(parsed["flags"].is_u64());
    assert!(parsed["modes_active"].is_array());
}

// ============================================================================
// 9. Builder Enforcement Under Panic
// ============================================================================

#[test]
#[should_panic(expected = "alternate screen requires raw mode")]
fn e2e_builder_rejects_alt_without_raw() {
    ModeBuilder::new().alt_screen();
}

#[test]
#[should_panic(expected = "mouse capture requires alternate screen")]
fn e2e_builder_rejects_mouse_without_alt() {
    ModeBuilder::new().raw().mouse();
}

#[test]
#[should_panic(expected = "bracketed paste requires alternate screen")]
fn e2e_builder_rejects_paste_without_alt() {
    ModeBuilder::new().raw().bracketed_paste();
}

#[test]
#[should_panic(expected = "focus events requires alternate screen")]
fn e2e_builder_rejects_focus_without_alt() {
    ModeBuilder::new().raw().focus_events();
}
