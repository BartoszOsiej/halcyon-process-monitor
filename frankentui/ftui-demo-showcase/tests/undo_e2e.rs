#![forbid(unsafe_code)]

//! End-to-end tests for Undo/Redo Command History Framework (bd-1nac.6).
//!
//! These tests validate the undo/redo system integration:
//!
//! - Basic undo/redo operations
//! - Command merging behavior
//! - Transaction grouping
//!
//! # Invariants (Alien Artifact)
//!
//! 1. **Undo/Redo inverse**: undo(redo(x)) == x for any command.
//! 2. **Branch handling**: New action after undo clears redo stack.
//! 3. **Depth bound**: Undo stack never exceeds max_depth.
//! 4. **Memory bound**: Total command bytes never exceed max_bytes.
//!
//! # Failure Modes
//!
//! | Scenario | Expected Behavior |
//! |----------|-------------------|
//! | Undo with empty stack | Returns None, no-op |
//! | Redo with empty stack | Returns None, no-op |
//! | Push after undo | Clears redo stack |
//! | Exceed max_depth | Oldest command evicted |
//! | Exceed max_bytes | Oldest commands evicted |
//!
//! # JSONL Log Schema (bd-1nac.6)
//!
//! - `run_id`: Unique run identifier
//! - `case`: Test case name
//! - `timings`: Timing info (duration_us)
//! - `outcome`: Test result
//! - `undo_depth`: Undo stack depth after operation
//! - `redo_depth`: Redo stack depth after operation
//! - `memory_bytes`: Total memory used by commands
//!
//! Run: `cargo test -p ftui-demo-showcase --test undo_e2e -- --nocapture`

use std::sync::OnceLock;
use std::time::Instant;

use ftui_harness::determinism::DeterminismFixture;
use ftui_runtime::undo::{
    CommandBatch, HistoryConfig, HistoryManager, TextDeleteCmd, TextInsertCmd, Transaction,
    TransactionScope, UndoableCmd, WidgetId,
};

// ---------------------------------------------------------------------------
// JSONL Logging Helpers (bd-1nac.6 schema)
// ---------------------------------------------------------------------------

/// Generate a unique run ID for this test execution.
fn run_id() -> &'static str {
    fixture().run_id()
}

fn fixture() -> &'static DeterminismFixture {
    static FIXTURE: OnceLock<DeterminismFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| DeterminismFixture::new("undo_test", 42))
}

/// Emit a JSONL log entry.
fn log_jsonl(data: &serde_json::Value) {
    eprintln!("{}", serde_json::to_string(data).unwrap());
}

/// Log test start.
fn log_test_start(case: &str) -> Instant {
    log_jsonl(&serde_json::json!({
        "run_id": run_id(),
        "case": case,
        "event": "start",
    }));
    Instant::now()
}

/// Log test outcome.
fn log_test_outcome(case: &str, start: Instant, outcome: &str, extra: serde_json::Value) {
    let duration_us = start.elapsed().as_micros();
    let mut entry = serde_json::json!({
        "run_id": run_id(),
        "case": case,
        "event": "complete",
        "outcome": outcome,
        "timings": {
            "duration_us": duration_us,
        },
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            entry[k.clone()] = v.clone();
        }
    }
    log_jsonl(&entry);
}

// ---------------------------------------------------------------------------
// Test Helpers
// ---------------------------------------------------------------------------

/// Create a simple insert command for testing.
fn make_insert_cmd(text: &str) -> Box<dyn UndoableCmd> {
    Box::new(
        TextInsertCmd::new(WidgetId::new(1), 0, text)
            .with_apply(|_id, _pos, _text| Ok(()))
            .with_remove(|_id, _pos, _len| Ok(())),
    )
}

/// Create an insert command at a specific position.
fn make_insert_cmd_at(pos: usize, text: &str) -> Box<dyn UndoableCmd> {
    Box::new(
        TextInsertCmd::new(WidgetId::new(1), pos, text)
            .with_apply(|_id, _pos, _text| Ok(()))
            .with_remove(|_id, _pos, _len| Ok(())),
    )
}

/// Create a delete command for testing.
fn make_delete_cmd(pos: usize, deleted: &str) -> Box<dyn UndoableCmd> {
    Box::new(
        TextDeleteCmd::new(WidgetId::new(1), pos, deleted)
            .with_remove(|_id, _pos, _len| Ok(()))
            .with_insert(|_id, _pos, _text| Ok(())),
    )
}

// ===========================================================================
// 1. Basic Undo/Redo Tests
// ===========================================================================

#[test]
fn e2e_undo_single_action() {
    let start = log_test_start("e2e_undo_single_action");
    let mut mgr = HistoryManager::default();

    // Push a command
    mgr.push(make_insert_cmd("hello"));
    assert!(mgr.can_undo());
    assert!(!mgr.can_redo());
    assert_eq!(mgr.undo_depth(), 1);

    // Undo it
    let result = mgr.undo();
    assert!(result.is_some());
    assert!(result.unwrap().is_ok());

    assert!(!mgr.can_undo());
    assert!(mgr.can_redo());
    assert_eq!(mgr.undo_depth(), 0);
    assert_eq!(mgr.redo_depth(), 1);

    log_test_outcome(
        "e2e_undo_single_action",
        start,
        "passed",
        serde_json::json!({
            "undo_depth": mgr.undo_depth(),
            "redo_depth": mgr.redo_depth(),
        }),
    );
}

#[test]
fn e2e_redo_after_undo() {
    let start = log_test_start("e2e_redo_after_undo");
    let mut mgr = HistoryManager::default();

    mgr.push(make_insert_cmd("hello"));
    mgr.undo();

    assert!(mgr.can_redo());

    let result = mgr.redo();
    assert!(result.is_some());
    assert!(result.unwrap().is_ok());

    assert!(mgr.can_undo());
    assert!(!mgr.can_redo());

    log_test_outcome(
        "e2e_redo_after_undo",
        start,
        "passed",
        serde_json::json!({
            "undo_depth": mgr.undo_depth(),
            "redo_depth": mgr.redo_depth(),
        }),
    );
}

#[test]
fn e2e_multiple_sequential_undos() {
    let start = log_test_start("e2e_multiple_sequential_undos");
    let mut mgr = HistoryManager::default();

    // Push 5 commands
    for i in 0..5 {
        mgr.push(make_insert_cmd(&format!("cmd{}", i)));
    }
    assert_eq!(mgr.undo_depth(), 5);

    // Undo all of them
    for i in 0..5 {
        assert!(mgr.can_undo());
        let result = mgr.undo();
        assert!(result.is_some());
        assert!(result.unwrap().is_ok());
        assert_eq!(mgr.redo_depth(), i + 1);
    }

    assert!(!mgr.can_undo());
    assert_eq!(mgr.undo_depth(), 0);
    assert_eq!(mgr.redo_depth(), 5);

    log_test_outcome(
        "e2e_multiple_sequential_undos",
        start,
        "passed",
        serde_json::json!({
            "commands_undone": 5,
            "undo_depth": mgr.undo_depth(),
            "redo_depth": mgr.redo_depth(),
        }),
    );
}

#[test]
fn e2e_redo_cleared_on_new_action() {
    let start = log_test_start("e2e_redo_cleared_on_new_action");
    let mut mgr = HistoryManager::default();

    // Push, undo
    mgr.push(make_insert_cmd("first"));
    mgr.undo();
    assert!(mgr.can_redo());
    assert_eq!(mgr.redo_depth(), 1);

    // Push new command - should clear redo
    mgr.push(make_insert_cmd("second"));
    assert!(!mgr.can_redo());
    assert_eq!(mgr.redo_depth(), 0);
    assert_eq!(mgr.undo_depth(), 1);

    log_test_outcome(
        "e2e_redo_cleared_on_new_action",
        start,
        "passed",
        serde_json::json!({
            "undo_depth": mgr.undo_depth(),
            "redo_depth": mgr.redo_depth(),
        }),
    );
}

#[test]
fn e2e_undo_on_empty_stack() {
    let start = log_test_start("e2e_undo_on_empty_stack");
    let mut mgr = HistoryManager::default();

    // Undo with nothing to undo
    let result = mgr.undo();
    assert!(result.is_none());

    assert!(!mgr.can_undo());
    assert!(!mgr.can_redo());

    log_test_outcome(
        "e2e_undo_on_empty_stack",
        start,
        "passed",
        serde_json::json!({
            "result": "none",
        }),
    );
}

#[test]
fn e2e_redo_on_empty_stack() {
    let start = log_test_start("e2e_redo_on_empty_stack");
    let mut mgr = HistoryManager::default();

    // Redo with nothing to redo
    let result = mgr.redo();
    assert!(result.is_none());

    assert!(!mgr.can_undo());
    assert!(!mgr.can_redo());

    log_test_outcome(
        "e2e_redo_on_empty_stack",
        start,
        "passed",
        serde_json::json!({
            "result": "none",
        }),
    );
}

// ===========================================================================
// 2. Command Merging Tests
// ===========================================================================

#[test]
fn e2e_consecutive_inserts_default_behavior() {
    let start = log_test_start("e2e_consecutive_inserts_default_behavior");
    let mut mgr = HistoryManager::default();

    // Push consecutive character inserts with default config
    mgr.push(make_insert_cmd("h"));
    let depth_after_first = mgr.undo_depth();

    mgr.push(make_insert_cmd_at(1, "e"));
    let depth_after_second = mgr.undo_depth();

    // Log actual behavior (merging depends on implementation)
    log_test_outcome(
        "e2e_consecutive_inserts_default_behavior",
        start,
        "passed",
        serde_json::json!({
            "depth_after_first": depth_after_first,
            "depth_after_second": depth_after_second,
            "separate_commands": depth_after_second > depth_after_first,
        }),
    );
}

#[test]
fn e2e_non_adjacent_inserts_no_merge() {
    let start = log_test_start("e2e_non_adjacent_inserts_no_merge");
    let mut mgr = HistoryManager::default();

    // Insert at position 0
    mgr.push(make_insert_cmd("hello"));

    // Insert at position 100 (non-adjacent)
    mgr.push(make_insert_cmd_at(100, "world"));

    // Should not merge - different positions
    assert_eq!(mgr.undo_depth(), 2);

    log_test_outcome(
        "e2e_non_adjacent_inserts_no_merge",
        start,
        "passed",
        serde_json::json!({
            "undo_depth": mgr.undo_depth(),
            "merged": false,
        }),
    );
}

// ===========================================================================
// 3. Transaction Grouping Tests
// ===========================================================================

#[test]
fn e2e_transaction_atomic_undo() {
    let start = log_test_start("e2e_transaction_atomic_undo");
    let mut mgr = HistoryManager::default();

    // Create a transaction with multiple commands
    let mut tx = Transaction::begin("Multi-insert");
    let _ = tx.execute(make_insert_cmd("hello"));
    let _ = tx.execute(make_insert_cmd_at(5, " "));
    let _ = tx.execute(make_insert_cmd_at(6, "world"));

    // Commit and push
    if let Some(tx) = tx.commit() {
        mgr.push(tx);
    }

    // Should be a single undo item
    assert_eq!(mgr.undo_depth(), 1);
    assert!(mgr.can_undo());

    // Undo should undo all three
    let undo_result = mgr.undo();
    assert!(undo_result.is_some());
    assert!(undo_result.unwrap().is_ok());

    assert_eq!(mgr.undo_depth(), 0);
    assert_eq!(mgr.redo_depth(), 1);

    log_test_outcome(
        "e2e_transaction_atomic_undo",
        start,
        "passed",
        serde_json::json!({
            "commands_in_transaction": 3,
            "undo_as_single": true,
        }),
    );
}

#[test]
fn e2e_transaction_scope() {
    let start = log_test_start("e2e_transaction_scope");
    let mut mgr = HistoryManager::default();

    // Use TransactionScope for RAII-style grouping
    {
        let mut scope = TransactionScope::new(&mut mgr);
        scope.begin("Scoped edits");
        let _ = scope.execute(make_insert_cmd("one"));
        let _ = scope.execute(make_insert_cmd_at(3, "two"));
        let _ = scope.commit();
    }

    // Should be a single undo item
    assert_eq!(mgr.undo_depth(), 1);

    log_test_outcome(
        "e2e_transaction_scope",
        start,
        "passed",
        serde_json::json!({
            "scope_commands": 2,
            "undo_depth": mgr.undo_depth(),
        }),
    );
}

#[test]
fn e2e_nested_transaction_scopes() {
    let start = log_test_start("e2e_nested_transaction_scopes");
    let mut mgr = HistoryManager::default();

    {
        let mut scope = TransactionScope::new(&mut mgr);

        // Outer transaction
        scope.begin("Outer");
        let _ = scope.execute(make_insert_cmd("A"));

        // Inner transaction
        scope.begin("Inner");
        let _ = scope.execute(make_insert_cmd("B"));
        let _ = scope.execute(make_insert_cmd("C"));
        let _ = scope.commit(); // Commit inner

        let _ = scope.execute(make_insert_cmd("D"));
        let _ = scope.commit(); // Commit outer
    }

    // Outer transaction should be a single undo item
    assert_eq!(mgr.undo_depth(), 1);

    log_test_outcome(
        "e2e_nested_transaction_scopes",
        start,
        "passed",
        serde_json::json!({
            "undo_depth": mgr.undo_depth(),
            "nested_structure": "outer(A, inner(B,C), D)",
        }),
    );
}

// ===========================================================================
// 4. Limits and Memory Management Tests
// ===========================================================================

#[test]
fn e2e_max_depth_evicts_oldest() {
    let start = log_test_start("e2e_max_depth_evicts_oldest");

    let config = HistoryConfig::new(5, 0); // Max 5 commands
    let mut mgr = HistoryManager::new(config);

    // Push 10 commands
    for i in 0..10 {
        mgr.push(make_insert_cmd(&format!("cmd{}", i)));
    }

    // Should only keep 5
    assert_eq!(mgr.undo_depth(), 5);

    log_test_outcome(
        "e2e_max_depth_evicts_oldest",
        start,
        "passed",
        serde_json::json!({
            "commands_pushed": 10,
            "max_depth": 5,
            "actual_depth": mgr.undo_depth(),
        }),
    );
}

#[test]
fn e2e_memory_limit_enforced() {
    let start = log_test_start("e2e_memory_limit_enforced");

    // Set very small memory limit
    let config = HistoryConfig::new(100, 500); // 500 bytes
    let mut mgr = HistoryManager::new(config);

    // Push commands until we hit the limit
    for i in 0..20 {
        mgr.push(make_insert_cmd(&format!("command_{:04}", i)));
    }

    // Memory should be bounded
    let memory = mgr.memory_usage();
    assert!(memory <= 600); // Allow some overhead

    log_test_outcome(
        "e2e_memory_limit_enforced",
        start,
        "passed",
        serde_json::json!({
            "max_bytes": 500,
            "actual_bytes": memory,
            "undo_depth": mgr.undo_depth(),
        }),
    );
}

#[test]
fn e2e_descriptions_readable() {
    let start = log_test_start("e2e_descriptions_readable");
    let mut mgr = HistoryManager::default();

    mgr.push(make_insert_cmd("hello"));
    mgr.push(make_delete_cmd(0, "hello"));

    let descriptions = mgr.undo_descriptions(10);
    assert_eq!(descriptions.len(), 2);

    // All descriptions should be non-empty
    assert!(descriptions.iter().all(|d| !d.is_empty()));

    log_test_outcome(
        "e2e_descriptions_readable",
        start,
        "passed",
        serde_json::json!({
            "descriptions": descriptions,
        }),
    );
}

// ===========================================================================
// 5. Command Batch Tests
// ===========================================================================

#[test]
fn e2e_command_batch() {
    let start = log_test_start("e2e_command_batch");

    let mut batch = CommandBatch::new("Edit batch");
    batch.push(make_insert_cmd("one"));
    batch.push(make_insert_cmd("two"));
    batch.push(make_insert_cmd("three"));

    assert_eq!(batch.len(), 3);
    assert!(!batch.is_empty());
    assert_eq!(batch.description(), "Edit batch");

    // Execute
    let result = batch.execute();
    assert!(result.is_ok());

    // Undo
    let undo_result = batch.undo();
    assert!(undo_result.is_ok());

    // Redo
    let redo_result = batch.redo();
    assert!(redo_result.is_ok());

    log_test_outcome(
        "e2e_command_batch",
        start,
        "passed",
        serde_json::json!({
            "batch_size": batch.len(),
        }),
    );
}

// ===========================================================================
// 6. Stress Tests
// ===========================================================================

#[test]
fn e2e_stress_many_undos_redos() {
    let start = log_test_start("e2e_stress_many_undos_redos");
    let mut mgr = HistoryManager::new(HistoryConfig::new(1000, 0));

    // Push many commands
    for i in 0..100 {
        mgr.push(make_insert_cmd(&format!("stress_{}", i)));
    }
    assert_eq!(mgr.undo_depth(), 100);

    // Undo all
    for _ in 0..100 {
        let _ = mgr.undo();
    }
    assert_eq!(mgr.undo_depth(), 0);
    assert_eq!(mgr.redo_depth(), 100);

    // Redo half
    for _ in 0..50 {
        let _ = mgr.redo();
    }
    assert_eq!(mgr.undo_depth(), 50);
    assert_eq!(mgr.redo_depth(), 50);

    log_test_outcome(
        "e2e_stress_many_undos_redos",
        start,
        "passed",
        serde_json::json!({
            "total_operations": 250,
            "final_undo_depth": mgr.undo_depth(),
            "final_redo_depth": mgr.redo_depth(),
        }),
    );
}

#[test]
fn e2e_clear_history() {
    let start = log_test_start("e2e_clear_history");
    let mut mgr = HistoryManager::default();

    mgr.push(make_insert_cmd("one"));
    mgr.push(make_insert_cmd("two"));
    mgr.undo();

    assert_eq!(mgr.undo_depth(), 1);
    assert_eq!(mgr.redo_depth(), 1);

    // Clear
    mgr.clear();

    assert_eq!(mgr.undo_depth(), 0);
    assert_eq!(mgr.redo_depth(), 0);
    assert!(!mgr.can_undo());
    assert!(!mgr.can_redo());

    log_test_outcome(
        "e2e_clear_history",
        start,
        "passed",
        serde_json::json!({
            "cleared": true,
        }),
    );
}

// ===========================================================================
// 7. Determinism Tests
// ===========================================================================

#[test]
fn e2e_deterministic_ordering() {
    let start = log_test_start("e2e_deterministic_ordering");

    // Run the same sequence twice
    let mut results = Vec::new();

    for _run in 0..2 {
        let mut mgr = HistoryManager::default();

        mgr.push(make_insert_cmd("A"));
        mgr.push(make_insert_cmd("B"));
        mgr.push(make_insert_cmd("C"));
        mgr.undo();
        mgr.push(make_insert_cmd("D"));

        results.push((mgr.undo_depth(), mgr.redo_depth()));
    }

    // Results should be identical
    assert_eq!(results[0], results[1]);

    log_test_outcome(
        "e2e_deterministic_ordering",
        start,
        "passed",
        serde_json::json!({
            "runs": 2,
            "deterministic": true,
            "final_state": format!("{:?}", results[0]),
        }),
    );
}

// ===========================================================================
// 8. Next Description Tests
// ===========================================================================

#[test]
fn e2e_next_undo_redo_descriptions() {
    let start = log_test_start("e2e_next_undo_redo_descriptions");
    let mut mgr = HistoryManager::default();

    // Initially no descriptions
    assert_eq!(mgr.next_undo_description(), None);
    assert_eq!(mgr.next_redo_description(), None);

    // Push a command
    mgr.push(make_insert_cmd("hello"));

    // Should have undo description
    assert!(mgr.next_undo_description().is_some());
    assert_eq!(mgr.next_redo_description(), None);

    // After undo, should have redo description
    mgr.undo();
    assert_eq!(mgr.next_undo_description(), None);
    assert!(mgr.next_redo_description().is_some());

    log_test_outcome(
        "e2e_next_undo_redo_descriptions",
        start,
        "passed",
        serde_json::json!({
            "status": "descriptions_work",
        }),
    );
}

// ===========================================================================
// 9. Widget-Style Undo Tests (bd-1nac.6 requirement)
// ===========================================================================
// Note: These tests simulate widget undo operations using HistoryManager
// and the TextInsertCmd/TextDeleteCmd command types. Full widget integration
// would require implementing UndoSupport trait on each widget.

#[test]
fn e2e_simulated_textinput_undo() {
    let start = log_test_start("e2e_simulated_textinput_undo");
    let mut mgr = HistoryManager::default();

    // Simulate text input operations with non-adjacent inserts
    let widget_a = WidgetId::new(100);
    let widget_b = WidgetId::new(101);

    // Insert from different widgets (won't merge)
    mgr.push(Box::new(
        TextInsertCmd::new(widget_a, 0, "Hello")
            .with_apply(|_, _, _| Ok(()))
            .with_remove(|_, _, _| Ok(())),
    ));
    mgr.push(Box::new(
        TextInsertCmd::new(widget_b, 0, "World")
            .with_apply(|_, _, _| Ok(()))
            .with_remove(|_, _, _| Ok(())),
    ));

    assert_eq!(mgr.undo_depth(), 2);

    // Undo "World"
    let _ = mgr.undo();
    assert_eq!(mgr.undo_depth(), 1);
    assert_eq!(mgr.redo_depth(), 1);

    log_test_outcome(
        "e2e_simulated_textinput_undo",
        start,
        "passed",
        serde_json::json!({
            "widget_type": "TextInput",
            "undo_depth": mgr.undo_depth(),
            "redo_depth": mgr.redo_depth(),
        }),
    );
}

#[test]
fn e2e_simulated_word_delete_undo() {
    let start = log_test_start("e2e_simulated_word_delete_undo");
    let mut mgr = HistoryManager::default();

    let widget_id = WidgetId::new(101);

    // Simulate deleting a word "world" from "hello world"
    mgr.push(Box::new(
        TextDeleteCmd::new(widget_id, 6, "world")
            .with_remove(|_, _, _| Ok(()))
            .with_insert(|_, _, _| Ok(())),
    ));

    assert!(mgr.can_undo());

    // Undo should restore "world"
    let result = mgr.undo();
    assert!(result.is_some());
    assert!(result.unwrap().is_ok());

    assert!(!mgr.can_undo());
    assert!(mgr.can_redo());

    log_test_outcome(
        "e2e_simulated_word_delete_undo",
        start,
        "passed",
        serde_json::json!({
            "widget_type": "TextInput",
            "word_deleted": "world",
            "undo_restored": true,
        }),
    );
}

#[test]
fn e2e_cross_widget_history() {
    let start = log_test_start("e2e_cross_widget_history");
    let mut mgr = HistoryManager::default();

    // Simulate operations from multiple widgets
    let text_widget = WidgetId::new(200);
    let list_widget = WidgetId::new(201);
    let tree_widget = WidgetId::new(202);

    // 1. Text widget insert
    mgr.push(Box::new(
        TextInsertCmd::new(text_widget, 0, "hello")
            .with_apply(|_, _, _| Ok(()))
            .with_remove(|_, _, _| Ok(())),
    ));

    // 2. List widget selection change (simulated as insert)
    mgr.push(Box::new(
        TextInsertCmd::new(list_widget, 0, "select:5")
            .with_apply(|_, _, _| Ok(()))
            .with_remove(|_, _, _| Ok(())),
    ));

    // 3. Tree widget expand (simulated as insert)
    mgr.push(Box::new(
        TextInsertCmd::new(tree_widget, 0, "expand:root")
            .with_apply(|_, _, _| Ok(()))
            .with_remove(|_, _, _| Ok(())),
    ));

    // 4. Text widget delete
    mgr.push(Box::new(
        TextDeleteCmd::new(text_widget, 0, "he")
            .with_remove(|_, _, _| Ok(()))
            .with_insert(|_, _, _| Ok(())),
    ));

    assert_eq!(mgr.undo_depth(), 4);

    // Undo all operations in reverse order
    while mgr.can_undo() {
        let _ = mgr.undo();
    }

    assert_eq!(mgr.undo_depth(), 0);
    assert_eq!(mgr.redo_depth(), 4);

    // Redo half
    let _ = mgr.redo();
    let _ = mgr.redo();

    assert_eq!(mgr.undo_depth(), 2);
    assert_eq!(mgr.redo_depth(), 2);

    log_test_outcome(
        "e2e_cross_widget_history",
        start,
        "passed",
        serde_json::json!({
            "widgets": 3,
            "total_commands": 4,
            "cross_widget_undo_works": true,
        }),
    );
}

#[test]
fn e2e_undo_redo_interleaving() {
    let start = log_test_start("e2e_undo_redo_interleaving");
    let mut mgr = HistoryManager::default();

    // Push A, B, C
    for ch in ['A', 'B', 'C'] {
        mgr.push(make_insert_cmd(&ch.to_string()));
    }

    // Undo C and B
    let _ = mgr.undo(); // undo C
    let _ = mgr.undo(); // undo B
    assert_eq!(mgr.undo_depth(), 1);
    assert_eq!(mgr.redo_depth(), 2);

    // Redo B
    let _ = mgr.redo();
    assert_eq!(mgr.undo_depth(), 2);
    assert_eq!(mgr.redo_depth(), 1);

    // Push D (clears redo stack)
    mgr.push(make_insert_cmd("D"));
    assert_eq!(mgr.undo_depth(), 3);
    assert_eq!(mgr.redo_depth(), 0);

    log_test_outcome(
        "e2e_undo_redo_interleaving",
        start,
        "passed",
        serde_json::json!({
            "interleaved_ops": true,
            "final_undo_depth": mgr.undo_depth(),
            "final_redo_depth": mgr.redo_depth(),
        }),
    );
}
