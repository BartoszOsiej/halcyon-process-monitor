#![forbid(unsafe_code)]

//! Transaction support for grouping multiple commands atomically.
//!
//! Transactions allow multiple operations to be grouped together and
//! treated as a single undoable unit. If any operation fails, all
//! previous operations in the transaction are rolled back.
//!
//! # Usage
//!
//! ```ignore
//! use ftui_runtime::undo::{HistoryManager, Transaction};
//!
//! let mut history = HistoryManager::default();
//!
//! // Begin a transaction
//! let mut txn = Transaction::begin("Format Document");
//!
//! // Add commands to the transaction
//! txn.push(normalize_whitespace_cmd)?;
//! txn.push(fix_indentation_cmd)?;
//! txn.push(sort_imports_cmd)?;
//!
//! // Commit the transaction to history
//! history.push(txn.commit());
//! ```
//!
//! # Nested Transactions
//!
//! Transactions can be nested using `TransactionScope`:
//!
//! ```ignore
//! let mut scope = TransactionScope::new(&mut history);
//!
//! // Outer transaction
//! scope.begin("Refactor");
//!
//! // Inner transaction
//! scope.begin("Rename Variable");
//! scope.execute(rename_cmd)?;
//! scope.commit()?;
//!
//! // More outer work
//! scope.execute(move_function_cmd)?;
//! scope.commit()?;
//! ```
//!
//! # Invariants
//!
//! 1. A committed transaction acts as a single command in history
//! 2. Rollback undoes all executed commands in reverse order
//! 3. Nested transactions must be committed/rolled back in order
//! 4. Empty transactions produce no history entry

use std::fmt;

use super::command::{CommandBatch, CommandError, CommandResult, UndoableCmd};
use super::history::HistoryManager;

/// Builder for creating a group of commands as a single transaction.
///
/// Commands added to a transaction are executed immediately. If any
/// command fails, all previously executed commands are rolled back.
///
/// When committed, the transaction becomes a single entry in history
/// that can be undone/redone atomically.
pub struct Transaction {
    /// The underlying command batch.
    batch: CommandBatch,
    /// Number of commands that have been successfully executed.
    executed_count: usize,
    /// Whether the transaction has been committed or rolled back.
    finalized: bool,
}

impl fmt::Debug for Transaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transaction")
            .field("description", &self.batch.description())
            .field("command_count", &self.batch.len())
            .field("executed_count", &self.executed_count)
            .field("finalized", &self.finalized)
            .finish()
    }
}

impl Transaction {
    /// Begin a new transaction with the given description.
    #[must_use]
    pub fn begin(description: impl Into<String>) -> Self {
        Self {
            batch: CommandBatch::new(description),
            executed_count: 0,
            finalized: false,
        }
    }

    /// Execute a command and add it to the transaction.
    ///
    /// The command is executed immediately. If it fails, all previously
    /// executed commands are rolled back and the error is returned.
    ///
    /// # Errors
    ///
    /// Returns error if the command fails to execute.
    pub fn execute(&mut self, mut cmd: Box<dyn UndoableCmd>) -> CommandResult {
        if self.finalized {
            return Err(CommandError::InvalidState(
                "transaction already finalized".to_string(),
            ));
        }

        // Execute the command
        if let Err(e) = cmd.execute() {
            // Rollback on failure
            self.rollback();
            return Err(e);
        }

        // Add to batch (already executed)
        self.executed_count += 1;
        self.batch.push_executed(cmd);
        Ok(())
    }

    /// Add a pre-executed command to the transaction.
    ///
    /// Use this when the command has already been executed externally.
    /// The command will be undone on rollback and redone on redo.
    ///
    /// # Errors
    ///
    /// Returns error if the transaction is already finalized.
    pub fn add_executed(&mut self, cmd: Box<dyn UndoableCmd>) -> CommandResult {
        if self.finalized {
            return Err(CommandError::InvalidState(
                "transaction already finalized".to_string(),
            ));
        }

        self.executed_count += 1;
        self.batch.push_executed(cmd);
        Ok(())
    }

    /// Commit the transaction, returning it as a single undoable command.
    ///
    /// Returns `None` if the transaction is empty.
    #[must_use]
    pub fn commit(mut self) -> Option<Box<dyn UndoableCmd>> {
        // Rollback/drop already finalized this transaction; never emit history.
        if self.finalized {
            return None;
        }
        self.finalized = true;

        if self.batch.is_empty() {
            None
        } else {
            // Take ownership of the batch, replacing with an empty one.
            // This works because Drop only rolls back if not finalized,
            // and we just set finalized = true.
            let batch = std::mem::replace(&mut self.batch, CommandBatch::new(""));
            Some(Box::new(batch))
        }
    }

    /// Roll back all executed commands in the transaction.
    ///
    /// This undoes all commands in reverse order. After rollback,
    /// the transaction is finalized and cannot be used further.
    pub fn rollback(&mut self) {
        if self.finalized {
            return;
        }

        // Rollback already happens in batch.undo(), but we need to
        // manually track that we're rolling back here
        // Since commands are in the batch but haven't been "undone" via
        // the batch's undo mechanism, we need to undo them directly.

        // The batch stores commands but doesn't track execution state
        // the same way we do. We need to undo the executed commands.
        // Since we can't easily access individual commands in the batch,
        // we rely on the batch's undo mechanism.

        // Mark as finalized before undo to prevent re-entry
        self.finalized = true;

        // If we have executed commands, undo them via the batch
        if self.executed_count > 0 {
            // The batch's undo will undo all commands
            let _ = self.batch.undo();
            self.executed_count = 0;
        }
    }

    /// Check if the transaction is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.batch.is_empty()
    }

    /// Get the number of commands in the transaction.
    #[must_use]
    pub fn len(&self) -> usize {
        self.batch.len()
    }

    /// Get the transaction description.
    #[must_use]
    pub fn description(&self) -> &str {
        self.batch.description()
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        // If transaction wasn't finalized, auto-rollback
        if !self.finalized {
            self.rollback();
        }
    }
}

/// Scope-based transaction manager for nested transactions.
///
/// Provides a stack-based interface for managing nested transactions.
/// Each `begin()` pushes a new transaction, and `commit()` or `rollback()`
/// pops and finalizes it.
pub struct TransactionScope<'a> {
    /// Reference to the history manager.
    history: &'a mut HistoryManager,
    /// Stack of active transactions.
    stack: Vec<Transaction>,
}

impl<'a> TransactionScope<'a> {
    /// Create a new transaction scope.
    #[must_use]
    pub fn new(history: &'a mut HistoryManager) -> Self {
        Self {
            history,
            stack: Vec::new(),
        }
    }

    /// Begin a new nested transaction.
    pub fn begin(&mut self, description: impl Into<String>) {
        self.stack.push(Transaction::begin(description));
    }

    /// Execute a command in the current transaction.
    ///
    /// If no transaction is active, the command is executed and added
    /// directly to history.
    ///
    /// # Errors
    ///
    /// Returns error if the command fails.
    pub fn execute(&mut self, cmd: Box<dyn UndoableCmd>) -> CommandResult {
        if let Some(txn) = self.stack.last_mut() {
            txn.execute(cmd)
        } else {
            // No active transaction, execute directly
            let mut cmd = cmd;
            cmd.execute()?;
            self.history.push(cmd);
            Ok(())
        }
    }

    /// Commit the current transaction.
    ///
    /// If nested, the committed transaction is added to the parent.
    /// If at top level, it's added to history.
    ///
    /// # Errors
    ///
    /// Returns error if no transaction is active.
    pub fn commit(&mut self) -> CommandResult {
        let txn = self
            .stack
            .pop()
            .ok_or_else(|| CommandError::InvalidState("no active transaction".to_string()))?;

        if let Some(cmd) = txn.commit() {
            if let Some(parent) = self.stack.last_mut() {
                // Add to parent transaction as pre-executed
                parent.add_executed(cmd)?;
            } else {
                // Add to history
                self.history.push(cmd);
            }
        }

        Ok(())
    }

    /// Roll back the current transaction.
    ///
    /// # Errors
    ///
    /// Returns error if no transaction is active.
    pub fn rollback(&mut self) -> CommandResult {
        let mut txn = self
            .stack
            .pop()
            .ok_or_else(|| CommandError::InvalidState("no active transaction".to_string()))?;

        txn.rollback();
        Ok(())
    }

    /// Check if there are active transactions.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.stack.is_empty()
    }

    /// Get the current nesting depth.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

impl Drop for TransactionScope<'_> {
    fn drop(&mut self) {
        // Auto-rollback any uncommitted transactions
        while let Some(mut txn) = self.stack.pop() {
            txn.rollback();
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::command::{TextInsertCmd, WidgetId};
    use crate::undo::history::HistoryConfig;
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Helper to create a test command that is **pre-executed**.
    ///
    /// Use this with APIs that expect an executed command (e.g. `add_executed`).
    fn make_cmd(buffer: Arc<Mutex<String>>, text: &str) -> Box<dyn UndoableCmd> {
        let b1 = buffer.clone();
        let b2 = buffer.clone();
        let text = text.to_string();
        let text_clone = text.clone();

        let mut cmd = TextInsertCmd::new(WidgetId::new(1), 0, text)
            .with_apply(move |_, _, txt| {
                let mut buf = b1.lock().unwrap();
                buf.push_str(txt);
                Ok(())
            })
            .with_remove(move |_, _, _| {
                let mut buf = b2.lock().unwrap();
                let new_len = buf.len().saturating_sub(text_clone.len());
                buf.truncate(new_len);
                Ok(())
            });

        cmd.execute().unwrap();
        Box::new(cmd)
    }

    #[test]
    fn test_empty_transaction() {
        let txn = Transaction::begin("Empty");
        assert!(txn.is_empty());
        assert_eq!(txn.len(), 0);
        assert!(txn.commit().is_none());
    }

    #[test]
    fn test_single_command_transaction() {
        let buffer = Arc::new(Mutex::new(String::new()));

        let mut txn = Transaction::begin("Single");
        txn.add_executed(make_cmd(buffer.clone(), "hello")).unwrap();

        assert_eq!(txn.len(), 1);

        let cmd = txn.commit();
        assert!(cmd.is_some());
    }

    #[test]
    fn test_transaction_rollback() {
        let buffer = Arc::new(Mutex::new(String::new()));

        let mut txn = Transaction::begin("Rollback Test");
        txn.add_executed(make_cmd(buffer.clone(), "hello")).unwrap();
        txn.add_executed(make_cmd(buffer.clone(), " world"))
            .unwrap();

        assert_eq!(*buffer.lock().unwrap(), "hello world");

        txn.rollback();

        // Buffer should be back to empty after rollback
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_transaction_commit_to_history() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        let mut txn = Transaction::begin("Commit Test");
        txn.add_executed(make_cmd(buffer.clone(), "a")).unwrap();
        txn.add_executed(make_cmd(buffer.clone(), "b")).unwrap();

        if let Some(cmd) = txn.commit() {
            history.push(cmd);
        }

        assert_eq!(history.undo_depth(), 1);
        assert!(history.can_undo());
    }

    #[test]
    fn test_transaction_undo_redo() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        let mut txn = Transaction::begin("Undo/Redo Test");
        txn.add_executed(make_cmd(buffer.clone(), "hello")).unwrap();
        txn.add_executed(make_cmd(buffer.clone(), " world"))
            .unwrap();

        if let Some(cmd) = txn.commit() {
            history.push(cmd);
        }

        assert_eq!(*buffer.lock().unwrap(), "hello world");

        // Undo the entire transaction
        history.undo();
        assert_eq!(*buffer.lock().unwrap(), "");

        // Redo the entire transaction
        history.redo();
        assert_eq!(*buffer.lock().unwrap(), "hello world");
    }

    #[test]
    fn test_scope_basic() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);
            scope.begin("Scope Test");

            scope.execute(make_scope_cmd(buffer.clone(), "a")).unwrap();
            scope.execute(make_scope_cmd(buffer.clone(), "b")).unwrap();

            scope.commit().unwrap();
        }

        assert_eq!(history.undo_depth(), 1);
        assert_eq!(*buffer.lock().unwrap(), "ab");
    }

    #[test]
    fn test_scope_nested() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            // Outer transaction
            scope.begin("Outer");
            scope
                .execute(make_scope_cmd(buffer.clone(), "outer1"))
                .unwrap();

            // Inner transaction
            scope.begin("Inner");
            scope
                .execute(make_scope_cmd(buffer.clone(), "inner"))
                .unwrap();
            scope.commit().unwrap();

            scope
                .execute(make_scope_cmd(buffer.clone(), "outer2"))
                .unwrap();
            scope.commit().unwrap();
        }

        // Both transactions committed as one (nested was added to parent)
        assert_eq!(history.undo_depth(), 1);
        assert_eq!(*buffer.lock().unwrap(), "outer1innerouter2");
    }

    #[test]
    fn test_scope_rollback() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);
            scope.begin("Rollback");

            scope.execute(make_scope_cmd(buffer.clone(), "a")).unwrap();
            scope.execute(make_scope_cmd(buffer.clone(), "b")).unwrap();

            scope.rollback().unwrap();
        }

        // Nothing should be in history
        assert_eq!(history.undo_depth(), 0);
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_scope_auto_rollback_on_drop() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);
            scope.begin("Will be dropped");
            scope
                .execute(make_scope_cmd(buffer.clone(), "test"))
                .unwrap();
            // scope drops without commit
        }

        // Should have auto-rolled back
        assert_eq!(history.undo_depth(), 0);
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_scope_depth() {
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        let mut scope = TransactionScope::new(&mut history);
        assert_eq!(scope.depth(), 0);
        assert!(!scope.is_active());

        scope.begin("Level 1");
        assert_eq!(scope.depth(), 1);
        assert!(scope.is_active());

        scope.begin("Level 2");
        assert_eq!(scope.depth(), 2);

        scope.commit().unwrap();
        assert_eq!(scope.depth(), 1);

        scope.commit().unwrap();
        assert_eq!(scope.depth(), 0);
        assert!(!scope.is_active());
    }

    #[test]
    fn test_transaction_description() {
        let txn = Transaction::begin("My Transaction");
        assert_eq!(txn.description(), "My Transaction");
    }

    #[test]
    fn test_finalized_transaction_rejects_commands() {
        let buffer = Arc::new(Mutex::new(String::new()));

        let mut txn = Transaction::begin("Finalized");
        txn.rollback();

        let result = txn.add_executed(make_cmd(buffer, "test"));
        assert!(result.is_err());
    }

    #[test]
    fn test_transaction_execute_method() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let b1 = buffer.clone();
        let b2 = buffer.clone();

        let cmd = TextInsertCmd::new(WidgetId::new(1), 0, "exec")
            .with_apply(move |_, _, txt| {
                let mut buf = b1.lock().unwrap();
                buf.push_str(txt);
                Ok(())
            })
            .with_remove(move |_, _, _| {
                let mut buf = b2.lock().unwrap();
                buf.drain(..4);
                Ok(())
            });

        let mut txn = Transaction::begin("Execute Test");
        txn.execute(Box::new(cmd)).unwrap();
        assert_eq!(txn.len(), 1);
        assert_eq!(*buffer.lock().unwrap(), "exec");
    }

    #[test]
    fn test_transaction_finalized_rejects_execute() {
        let buffer = Arc::new(Mutex::new(String::new()));

        let mut txn = Transaction::begin("Finalized");
        txn.rollback();

        let result = txn.execute(make_scope_cmd(buffer, "test"));
        assert!(result.is_err());
    }

    #[test]
    fn test_commit_after_rollback_returns_none() {
        let buffer = Arc::new(Mutex::new(String::new()));

        let mut txn = Transaction::begin("Rollback then commit");
        txn.add_executed(make_cmd(buffer.clone(), "a")).unwrap();
        txn.rollback();

        assert!(txn.commit().is_none());
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_scope_commit_after_execute_failure_does_not_push_rolled_back_batch() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        let failing_cmd = TextInsertCmd::new(WidgetId::new(1), 0, "boom")
            .with_apply(move |_, _, _| Err(CommandError::Other("boom".to_string())))
            .with_remove(move |_, _, _| Ok(()));

        {
            let mut scope = TransactionScope::new(&mut history);
            scope.begin("Failure path");
            let b_ok_apply = buffer.clone();
            let b_ok_remove = buffer.clone();
            let ok_cmd = TextInsertCmd::new(WidgetId::new(1), 0, "ok")
                .with_apply(move |_, _, txt| {
                    let mut buf = b_ok_apply.lock().unwrap();
                    buf.push_str(txt);
                    Ok(())
                })
                .with_remove(move |_, _, _| {
                    let mut buf = b_ok_remove.lock().unwrap();
                    buf.drain(..2);
                    Ok(())
                });
            scope.execute(Box::new(ok_cmd)).unwrap();
            assert!(scope.execute(Box::new(failing_cmd)).is_err());
            // This used to leak the rolled-back batch into history.
            scope.commit().unwrap();
        }

        assert_eq!(history.undo_depth(), 0);
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_scope_execute_without_transaction() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);
            // Execute without begin() - should go directly to history
            scope
                .execute(make_scope_cmd(buffer.clone(), "direct"))
                .unwrap();
        }

        assert_eq!(history.undo_depth(), 1);
        assert_eq!(*buffer.lock().unwrap(), "direct");
    }

    #[test]
    fn test_scope_commit_without_begin_errors() {
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        let mut scope = TransactionScope::new(&mut history);
        let result = scope.commit();
        assert!(result.is_err());
    }

    #[test]
    fn test_scope_rollback_without_begin_errors() {
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        let mut scope = TransactionScope::new(&mut history);
        let result = scope.rollback();
        assert!(result.is_err());
    }

    #[test]
    fn test_transaction_multi_command_rollback_order() {
        let buffer = Arc::new(Mutex::new(String::new()));

        let mut txn = Transaction::begin("Multi Rollback");
        txn.add_executed(make_cmd(buffer.clone(), "a")).unwrap();
        txn.add_executed(make_cmd(buffer.clone(), "b")).unwrap();
        txn.add_executed(make_cmd(buffer.clone(), "c")).unwrap();

        assert_eq!(*buffer.lock().unwrap(), "abc");
        txn.rollback();
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_transaction_debug_impl() {
        let txn = Transaction::begin("Debug Test");
        let s = format!("{txn:?}");
        assert!(s.contains("Transaction"));
        assert!(s.contains("Debug Test"));
    }

    // ====================================================================
    // Additional coverage: double rollback, scope edge cases
    // ====================================================================

    #[test]
    fn test_rollback_is_idempotent() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut txn = Transaction::begin("Double Rollback");
        txn.add_executed(make_cmd(buffer.clone(), "x")).unwrap();

        txn.rollback();
        assert_eq!(*buffer.lock().unwrap(), "");

        // Second rollback is a no-op (finalized)
        txn.rollback();
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_rollback_empty_transaction() {
        let mut txn = Transaction::begin("Empty Rollback");
        txn.rollback();
        // Should not panic, nothing to undo
        assert!(txn.commit().is_none());
    }

    #[test]
    fn test_scope_drop_with_multiple_uncommitted() {
        let mut history = HistoryManager::new(HistoryConfig::unlimited());
        let buf_a = Arc::new(Mutex::new(String::new()));
        let buf_b = Arc::new(Mutex::new(String::new()));

        {
            let mut scope = TransactionScope::new(&mut history);
            scope.begin("Outer");
            // Use separate buffers to avoid cross-transaction interference
            scope.execute(make_scope_cmd(buf_a.clone(), "a")).unwrap();

            scope.begin("Inner");
            scope.execute(make_scope_cmd(buf_b.clone(), "b")).unwrap();

            // Drop without committing either transaction
        }

        // Both should have been rolled back — nothing in history
        assert_eq!(history.undo_depth(), 0);
        assert_eq!(*buf_a.lock().unwrap(), "");
        assert_eq!(*buf_b.lock().unwrap(), "");
    }

    #[test]
    fn test_scope_inner_rollback_outer_continues() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            // Outer transaction
            scope.begin("Outer");
            scope
                .execute(make_scope_cmd(buffer.clone(), "outer"))
                .unwrap();

            // Inner transaction
            scope.begin("Inner");
            scope
                .execute(make_scope_cmd(buffer.clone(), "inner"))
                .unwrap();
            scope.rollback().unwrap(); // Roll back inner only

            assert_eq!(scope.depth(), 1); // Outer still active
            assert_eq!(*buffer.lock().unwrap(), "outer");

            // Commit outer
            scope.commit().unwrap();
        }

        assert_eq!(history.undo_depth(), 1);
    }

    #[test]
    fn test_scope_commit_empty_inner_txn() {
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);
            scope.begin("Outer");
            scope.begin("Empty Inner");
            scope.commit().unwrap(); // Empty inner commits as None
            scope.commit().unwrap(); // Outer commits as empty too
        }

        // Empty transactions produce no history entry
        assert_eq!(history.undo_depth(), 0);
    }

    #[test]
    fn test_transaction_execute_failure_rolls_back_prior() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let b1 = buffer.clone();
        let b2 = buffer.clone();

        let ok_cmd = TextInsertCmd::new(WidgetId::new(1), 0, "ok")
            .with_apply(move |_, _, txt| {
                let mut buf = b1.lock().unwrap();
                buf.push_str(txt);
                Ok(())
            })
            .with_remove(move |_, _, _| {
                let mut buf = b2.lock().unwrap();
                buf.drain(..2);
                Ok(())
            });

        // A command that fails on execute (no apply callback)
        let fail_cmd = TextInsertCmd::new(WidgetId::new(1), 2, "fail");

        let mut txn = Transaction::begin("Execute Failure");
        txn.execute(Box::new(ok_cmd)).unwrap();
        assert_eq!(*buffer.lock().unwrap(), "ok");

        // This should fail and rollback the "ok" command
        let result = txn.execute(Box::new(fail_cmd));
        assert!(result.is_err());
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_transaction_is_empty_after_add() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut txn = Transaction::begin("Not Empty");
        assert!(txn.is_empty());

        txn.add_executed(make_cmd(buffer, "x")).unwrap();
        assert!(!txn.is_empty());
    }

    #[test]
    fn test_scope_execute_failure_without_txn() {
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        // Execute without begin() with a failing command
        let fail_cmd = TextInsertCmd::new(WidgetId::new(1), 0, "fail");
        // No callbacks, so execute will fail

        {
            let mut scope = TransactionScope::new(&mut history);
            let result = scope.execute(Box::new(fail_cmd));
            assert!(result.is_err());
        }
        assert_eq!(history.undo_depth(), 0);
    }

    // ====================================================================
    // Additional coverage: sequential scopes, deep nesting, edge cases
    // (bd-1o6q1)
    // ====================================================================

    #[test]
    fn test_scope_sequential_transactions() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            // First transaction
            scope.begin("First");
            scope.execute(make_scope_cmd(buffer.clone(), "a")).unwrap();
            scope.commit().unwrap();

            // Second transaction in same scope
            scope.begin("Second");
            scope.execute(make_scope_cmd(buffer.clone(), "b")).unwrap();
            scope.commit().unwrap();
        }

        // Both should be separate history entries
        assert_eq!(history.undo_depth(), 2);
    }

    #[test]
    fn test_scope_three_level_nesting() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            scope.begin("Level 1");
            scope.execute(make_scope_cmd(buffer.clone(), "a")).unwrap();

            scope.begin("Level 2");
            scope.execute(make_scope_cmd(buffer.clone(), "b")).unwrap();

            scope.begin("Level 3");
            scope.execute(make_scope_cmd(buffer.clone(), "c")).unwrap();
            assert_eq!(scope.depth(), 3);

            scope.commit().unwrap();
            scope.commit().unwrap();
            scope.commit().unwrap();
        }

        assert_eq!(history.undo_depth(), 1);
        assert_eq!(*buffer.lock().unwrap(), "abc");

        history.undo();
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_scope_alternating_commit_rollback() {
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            // First: commit
            scope.begin("Committed");
            let buf1 = Arc::new(Mutex::new(String::new()));
            scope.execute(make_scope_cmd(buf1, "ok")).unwrap();
            scope.commit().unwrap();

            // Second: rollback
            scope.begin("Rolled back");
            let buf2 = Arc::new(Mutex::new(String::new()));
            scope.execute(make_scope_cmd(buf2, "no")).unwrap();
            scope.rollback().unwrap();

            // Third: commit
            scope.begin("Also committed");
            let buf3 = Arc::new(Mutex::new(String::new()));
            scope.execute(make_scope_cmd(buf3, "yes")).unwrap();
            scope.commit().unwrap();
        }

        // Two committed, one rolled back
        assert_eq!(history.undo_depth(), 2);
    }

    #[test]
    fn test_scope_rollback_then_new_transaction() {
        let buf_bad = Arc::new(Mutex::new(String::new()));
        let buf_good = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            scope.begin("Failed attempt");
            scope
                .execute(make_scope_cmd(buf_bad.clone(), "bad"))
                .unwrap();
            scope.rollback().unwrap();

            scope.begin("Retry");
            scope
                .execute(make_scope_cmd(buf_good.clone(), "good"))
                .unwrap();
            scope.commit().unwrap();
        }

        assert_eq!(history.undo_depth(), 1);
        assert_eq!(*buf_bad.lock().unwrap(), "");
        assert_eq!(*buf_good.lock().unwrap(), "good");
    }

    #[test]
    fn test_transaction_len_after_execute() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let b1 = buffer.clone();
        let b2 = buffer.clone();

        let cmd = TextInsertCmd::new(WidgetId::new(1), 0, "x")
            .with_apply(move |_, _, txt| {
                b1.lock().unwrap().push_str(txt);
                Ok(())
            })
            .with_remove(move |_, _, _| {
                b2.lock().unwrap().drain(..1);
                Ok(())
            });

        let mut txn = Transaction::begin("Len Test");
        assert_eq!(txn.len(), 0);

        txn.execute(Box::new(cmd)).unwrap();
        assert_eq!(txn.len(), 1);
    }

    #[test]
    fn test_transaction_many_commands() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut txn = Transaction::begin("Many Commands");

        for _ in 0..20 {
            txn.add_executed(make_cmd(buffer.clone(), ".")).unwrap();
        }

        assert_eq!(txn.len(), 20);
        assert_eq!(buffer.lock().unwrap().len(), 20);

        txn.rollback();
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    #[test]
    fn test_scope_execute_after_all_committed() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            // Transaction
            scope.begin("Txn");
            scope.execute(make_scope_cmd(buffer.clone(), "a")).unwrap();
            scope.commit().unwrap();

            // Direct execute (no active transaction) goes to history
            scope.execute(make_scope_cmd(buffer.clone(), "b")).unwrap();
        }

        // One from transaction, one from direct execute
        assert_eq!(history.undo_depth(), 2);
        assert_eq!(*buffer.lock().unwrap(), "ab");
    }

    #[test]
    fn test_scope_inner_commit_empty_outer_has_content() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            scope.begin("Outer with content");
            scope
                .execute(make_scope_cmd(buffer.clone(), "outer"))
                .unwrap();

            scope.begin("Empty inner");
            scope.commit().unwrap();

            scope.commit().unwrap();
        }

        assert_eq!(history.undo_depth(), 1);
        assert_eq!(*buffer.lock().unwrap(), "outer");
    }

    #[test]
    fn test_drop_transaction_without_finalize_rolls_back() {
        let buffer = Arc::new(Mutex::new(String::new()));

        {
            let mut txn = Transaction::begin("Will be dropped");
            txn.add_executed(make_cmd(buffer.clone(), "dropped"))
                .unwrap();
            assert_eq!(*buffer.lock().unwrap(), "dropped");
            // txn dropped here without commit or rollback
        }

        // Drop should auto-rollback
        assert_eq!(*buffer.lock().unwrap(), "");
    }

    /// Helper creating a command that is NOT pre-executed (for scope.execute).
    fn make_scope_cmd(buffer: Arc<Mutex<String>>, text: &str) -> Box<dyn UndoableCmd> {
        let b1 = buffer.clone();
        let b2 = buffer.clone();
        let text = text.to_string();
        let text_clone = text.clone();

        Box::new(
            TextInsertCmd::new(WidgetId::new(1), 0, text)
                .with_apply(move |_, _, txt| {
                    let mut buf = b1.lock().unwrap();
                    buf.push_str(txt);
                    Ok(())
                })
                .with_remove(move |_, _, _| {
                    let mut buf = b2.lock().unwrap();
                    let new_len = buf.len().saturating_sub(text_clone.len());
                    buf.truncate(new_len);
                    Ok(())
                }),
        )
    }

    #[test]
    fn test_scope_nested_rollback_preserves_outer() {
        let buf_outer = Arc::new(Mutex::new(String::new()));
        let buf_inner = Arc::new(Mutex::new(String::new()));
        let mut history = HistoryManager::new(HistoryConfig::unlimited());

        {
            let mut scope = TransactionScope::new(&mut history);

            scope.begin("Outer");
            scope
                .execute(make_scope_cmd(buf_outer.clone(), "A"))
                .unwrap();
            assert_eq!(*buf_outer.lock().unwrap(), "A");

            scope.begin("Inner (will rollback)");
            scope
                .execute(make_scope_cmd(buf_inner.clone(), "B"))
                .unwrap();
            assert_eq!(*buf_inner.lock().unwrap(), "B");

            scope.rollback().unwrap();
            assert_eq!(*buf_inner.lock().unwrap(), "");
            assert_eq!(*buf_outer.lock().unwrap(), "A");

            scope.commit().unwrap();
        }

        assert_eq!(history.undo_depth(), 1);
        assert_eq!(*buf_outer.lock().unwrap(), "A");
    }
}
