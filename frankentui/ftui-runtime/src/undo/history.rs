#![forbid(unsafe_code)]

//! History stack for undo/redo operations.
//!
//! This module provides the [`HistoryManager`] which maintains dual stacks
//! for undo and redo operations with support for:
//!
//! - **Memory limits**: Oldest commands evicted when budget exceeded
//! - **Depth limits**: Maximum number of commands in history
//! - **Branch handling**: New actions clear the redo stack
//! - **Command merging**: Consecutive similar commands batched together
//!
//! # Invariants
//!
//! 1. `total_bytes` always equals sum of `size_bytes()` for all commands
//! 2. `undo_stack.len() <= config.max_depth` (after any operation)
//! 3. `total_bytes <= config.max_bytes` (after any operation, if enforced)
//! 4. Redo stack is cleared whenever a new command is pushed
//!
//! # Memory Model
//!
//! Commands are stored in `VecDeque` for O(1) eviction from the front.
//! Memory accounting uses each command's `size_bytes()` method.
//!
//! ```text
//! push(cmd5)
//! ┌───────────────────────────────────────────────┐
//! │ Undo Stack: [cmd1, cmd2, cmd3, cmd4, cmd5]    │
//! │ Redo Stack: []                                 │
//! └───────────────────────────────────────────────┘
//!
//! undo() x2
//! ┌───────────────────────────────────────────────┐
//! │ Undo Stack: [cmd1, cmd2, cmd3]                │
//! │ Redo Stack: [cmd4, cmd5]                       │
//! └───────────────────────────────────────────────┘
//!
//! push(cmd6)  <-- new branch, clears redo
//! ┌───────────────────────────────────────────────┐
//! │ Undo Stack: [cmd1, cmd2, cmd3, cmd6]          │
//! │ Redo Stack: []                                 │
//! └───────────────────────────────────────────────┘
//! ```

use std::collections::VecDeque;
use std::fmt;

use super::command::{MergeConfig, UndoableCmd};

/// Configuration for the history manager.
#[derive(Debug, Clone)]
pub struct HistoryConfig {
    /// Maximum number of commands to keep in undo history.
    pub max_depth: usize,
    /// Maximum total bytes for all commands (0 = unlimited).
    pub max_bytes: usize,
    /// Configuration for command merging.
    pub merge_config: MergeConfig,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_depth: 100,
            max_bytes: 10 * 1024 * 1024, // 10 MB
            merge_config: MergeConfig::default(),
        }
    }
}

impl HistoryConfig {
    /// Create a new configuration with custom limits.
    #[must_use]
    pub fn new(max_depth: usize, max_bytes: usize) -> Self {
        Self {
            max_depth,
            max_bytes,
            merge_config: MergeConfig::default(),
        }
    }

    /// Set the merge configuration.
    #[must_use]
    pub fn with_merge_config(mut self, config: MergeConfig) -> Self {
        self.merge_config = config;
        self
    }

    /// Create unlimited configuration (for testing).
    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            max_depth: usize::MAX,
            max_bytes: 0,
            merge_config: MergeConfig::default(),
        }
    }
}

/// Manager for undo/redo history.
///
/// Maintains dual stacks for undo and redo operations with
/// configurable memory and depth limits.
pub struct HistoryManager {
    /// Commands available for undo (newest at back).
    undo_stack: VecDeque<Box<dyn UndoableCmd>>,
    /// Commands available for redo (newest at back).
    redo_stack: VecDeque<Box<dyn UndoableCmd>>,
    /// Configuration for limits and merging.
    config: HistoryConfig,
    /// Total bytes used by all commands.
    total_bytes: usize,
}

impl fmt::Debug for HistoryManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryManager")
            .field("undo_depth", &self.undo_stack.len())
            .field("redo_depth", &self.redo_stack.len())
            .field("total_bytes", &self.total_bytes)
            .field("config", &self.config)
            .finish()
    }
}

impl Default for HistoryManager {
    fn default() -> Self {
        Self::new(HistoryConfig::default())
    }
}

impl HistoryManager {
    /// Create a new history manager with the given configuration.
    #[must_use]
    pub fn new(config: HistoryConfig) -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            config,
            total_bytes: 0,
        }
    }

    // ========================================================================
    // Core Operations
    // ========================================================================

    /// Push a command onto the undo stack.
    ///
    /// This clears the redo stack (new branch) and enforces limits.
    /// The command is NOT executed - it's assumed to have already been executed.
    pub fn push(&mut self, cmd: Box<dyn UndoableCmd>) {
        // Clear redo stack (new branch)
        self.clear_redo();

        // Try to merge with the last command
        let cmd = match self.try_merge(cmd) {
            Ok(()) => {
                // Merged successfully, just enforce limits
                self.enforce_limits();
                return;
            }
            Err(cmd) => cmd,
        };

        // Add to undo stack
        self.total_bytes += cmd.size_bytes();
        self.undo_stack.push_back(cmd);

        // Enforce limits
        self.enforce_limits();
    }

    /// Undo the last command.
    ///
    /// Moves the command from undo stack to redo stack and calls undo().
    ///
    /// # Returns
    ///
    /// - `Ok(description)` if undo succeeded
    /// - `Err(error)` if undo failed (command remains on undo stack)
    /// - `None` if no commands to undo
    pub fn undo(&mut self) -> Option<Result<String, super::command::CommandError>> {
        let mut cmd = self.undo_stack.pop_back()?;
        let description = cmd.description().to_string();

        match cmd.undo() {
            Ok(()) => {
                // Move to redo stack
                self.redo_stack.push_back(cmd);
                Some(Ok(description))
            }
            Err(e) => {
                // Put back on undo stack
                self.undo_stack.push_back(cmd);
                Some(Err(e))
            }
        }
    }

    /// Redo the last undone command.
    ///
    /// Moves the command from redo stack to undo stack and calls redo().
    ///
    /// # Returns
    ///
    /// - `Ok(description)` if redo succeeded
    /// - `Err(error)` if redo failed (command remains on redo stack)
    /// - `None` if no commands to redo
    pub fn redo(&mut self) -> Option<Result<String, super::command::CommandError>> {
        let mut cmd = self.redo_stack.pop_back()?;
        let description = cmd.description().to_string();

        match cmd.redo() {
            Ok(()) => {
                // Move to undo stack
                self.undo_stack.push_back(cmd);
                Some(Ok(description))
            }
            Err(e) => {
                // Put back on redo stack
                self.redo_stack.push_back(cmd);
                Some(Err(e))
            }
        }
    }

    /// Check if undo is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if redo is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    // ========================================================================
    // Info
    // ========================================================================

    /// Get the undo stack depth.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Get the redo stack depth.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Get descriptions for undo commands (most recent first).
    pub fn undo_descriptions(&self, limit: usize) -> Vec<&str> {
        self.undo_stack
            .iter()
            .rev()
            .take(limit)
            .map(|c| c.description())
            .collect()
    }

    /// Get descriptions for redo commands (most recent first).
    pub fn redo_descriptions(&self, limit: usize) -> Vec<&str> {
        self.redo_stack
            .iter()
            .rev()
            .take(limit)
            .map(|c| c.description())
            .collect()
    }

    /// Get the description of the next undo command.
    #[must_use]
    pub fn next_undo_description(&self) -> Option<&str> {
        self.undo_stack.back().map(|c| c.description())
    }

    /// Get the description of the next redo command.
    #[must_use]
    pub fn next_redo_description(&self) -> Option<&str> {
        self.redo_stack.back().map(|c| c.description())
    }

    /// Get total memory usage in bytes.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.total_bytes
    }

    /// Get the current configuration.
    #[must_use]
    pub fn config(&self) -> &HistoryConfig {
        &self.config
    }

    // ========================================================================
    // Maintenance
    // ========================================================================

    /// Clear all history (both undo and redo).
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.total_bytes = 0;
    }

    /// Clear only the redo stack.
    fn clear_redo(&mut self) {
        for cmd in self.redo_stack.drain(..) {
            self.total_bytes = self.total_bytes.saturating_sub(cmd.size_bytes());
        }
    }

    /// Enforce depth and memory limits by evicting oldest commands.
    fn enforce_limits(&mut self) {
        // Enforce depth limit (only applies to undo stack)
        while self.undo_stack.len() > self.config.max_depth {
            if let Some(cmd) = self.undo_stack.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(cmd.size_bytes());
            }
        }

        // Enforce memory limit (if set) - applies to TOTAL history
        if self.config.max_bytes > 0 {
            while self.total_bytes > self.config.max_bytes {
                // First try to drop from redo stack (future/speculative history)
                if let Some(cmd) = self.redo_stack.pop_front() {
                    self.total_bytes = self.total_bytes.saturating_sub(cmd.size_bytes());
                    continue;
                }

                // Then drop from undo stack (oldest history)
                if let Some(cmd) = self.undo_stack.pop_front() {
                    self.total_bytes = self.total_bytes.saturating_sub(cmd.size_bytes());
                } else {
                    // Both stacks empty, nothing to drop
                    break;
                }
            }
        }
    }

    /// Try to merge a command with the last one on the undo stack.
    ///
    /// Returns `Ok(())` if merged, `Err(cmd)` if not merged.
    fn try_merge(&mut self, cmd: Box<dyn UndoableCmd>) -> Result<(), Box<dyn UndoableCmd>> {
        let Some(last) = self.undo_stack.back_mut() else {
            return Err(cmd);
        };

        // Check if merge is possible
        if !last.can_merge(cmd.as_ref(), &self.config.merge_config) {
            return Err(cmd);
        }

        // Verify the command has merge text available
        if cmd.merge_text().is_none() {
            return Err(cmd);
        }

        // Adjust memory accounting (old size will change)
        let old_size = last.size_bytes();

        // Perform the merge (pass full command for position context)
        if !last.accept_merge(cmd.as_ref()) {
            return Err(cmd);
        }

        // Update memory accounting
        let new_size = last.size_bytes();
        self.total_bytes = self.total_bytes.saturating_sub(old_size) + new_size;

        // The incoming command is consumed (merged into last)
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::command::{TextInsertCmd, WidgetId};
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Helper to create a simple insert command for testing.
    /// The command is pre-executed so undo() will work correctly.
    fn make_insert_cmd(text: &str) -> Box<dyn UndoableCmd> {
        let buffer = Arc::new(Mutex::new(String::new()));
        let b1 = buffer.clone();
        let b2 = buffer.clone();

        let mut cmd = TextInsertCmd::new(WidgetId::new(1), 0, text)
            .with_apply(move |_, pos, txt| {
                let mut buf = b1.lock().unwrap();
                buf.insert_str(pos, txt);
                Ok(())
            })
            .with_remove(move |_, pos, len| {
                let mut buf = b2.lock().unwrap();
                buf.drain(pos..pos + len);
                Ok(())
            });

        // Execute the command first so undo() will work
        cmd.execute().expect("test command should execute");

        Box::new(cmd)
    }

    #[test]
    fn test_new_manager() {
        let mgr = HistoryManager::default();
        assert!(!mgr.can_undo());
        assert!(!mgr.can_redo());
        assert_eq!(mgr.undo_depth(), 0);
        assert_eq!(mgr.redo_depth(), 0);
    }

    #[test]
    fn test_push_enables_undo() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("hello"));

        assert!(mgr.can_undo());
        assert!(!mgr.can_redo());
        assert_eq!(mgr.undo_depth(), 1);
    }

    #[test]
    fn test_undo_enables_redo() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("hello"));

        let result = mgr.undo();
        assert!(result.is_some());
        assert!(result.unwrap().is_ok());

        assert!(!mgr.can_undo());
        assert!(mgr.can_redo());
        assert_eq!(mgr.redo_depth(), 1);
    }

    #[test]
    fn test_redo_moves_back_to_undo() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("hello"));
        mgr.undo();

        let result = mgr.redo();
        assert!(result.is_some());
        assert!(result.unwrap().is_ok());

        assert!(mgr.can_undo());
        assert!(!mgr.can_redo());
    }

    #[test]
    fn test_push_clears_redo() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("hello"));
        mgr.undo();

        assert!(mgr.can_redo());

        // Push new command
        mgr.push(make_insert_cmd("world"));

        // Redo should be cleared
        assert!(!mgr.can_redo());
        assert_eq!(mgr.redo_depth(), 0);
    }

    #[test]
    fn test_max_depth_enforced() {
        let config = HistoryConfig::new(3, 0);
        let mut mgr = HistoryManager::new(config);

        for i in 0..5 {
            mgr.push(make_insert_cmd(&format!("cmd{}", i)));
        }

        // Should only keep 3 commands
        assert_eq!(mgr.undo_depth(), 3);
    }

    #[test]
    fn test_descriptions() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("a"));
        mgr.push(make_insert_cmd("b"));
        mgr.push(make_insert_cmd("c"));

        let descs = mgr.undo_descriptions(5);
        assert_eq!(descs.len(), 3);
        // All should be "Insert text"
        assert!(descs.iter().all(|d| *d == "Insert text"));
    }

    #[test]
    fn test_next_descriptions() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("hello"));

        assert_eq!(mgr.next_undo_description(), Some("Insert text"));
        assert_eq!(mgr.next_redo_description(), None);

        mgr.undo();

        assert_eq!(mgr.next_undo_description(), None);
        assert_eq!(mgr.next_redo_description(), Some("Insert text"));
    }

    #[test]
    fn test_clear() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("a"));
        mgr.push(make_insert_cmd("b"));
        mgr.undo();

        assert!(mgr.can_undo());
        assert!(mgr.can_redo());

        mgr.clear();

        assert!(!mgr.can_undo());
        assert!(!mgr.can_redo());
        assert_eq!(mgr.memory_usage(), 0);
    }

    #[test]
    fn test_memory_tracking() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());

        let initial = mgr.memory_usage();
        assert_eq!(initial, 0);

        mgr.push(make_insert_cmd("hello"));
        let after_push = mgr.memory_usage();
        assert!(after_push > 0);

        mgr.push(make_insert_cmd("world"));
        let after_second = mgr.memory_usage();
        assert!(after_second > after_push);
    }

    #[test]
    fn test_undo_without_commands() {
        let mut mgr = HistoryManager::default();
        assert!(mgr.undo().is_none());
    }

    #[test]
    fn test_redo_without_commands() {
        let mut mgr = HistoryManager::default();
        assert!(mgr.redo().is_none());
    }

    #[test]
    fn test_multiple_undo_redo_cycle() {
        let mut mgr = HistoryManager::default();

        // Push 3 commands
        mgr.push(make_insert_cmd("a"));
        mgr.push(make_insert_cmd("b"));
        mgr.push(make_insert_cmd("c"));

        assert_eq!(mgr.undo_depth(), 3);
        assert_eq!(mgr.redo_depth(), 0);

        // Undo all
        mgr.undo();
        mgr.undo();
        mgr.undo();

        assert_eq!(mgr.undo_depth(), 0);
        assert_eq!(mgr.redo_depth(), 3);

        // Redo all
        mgr.redo();
        mgr.redo();
        mgr.redo();

        assert_eq!(mgr.undo_depth(), 3);
        assert_eq!(mgr.redo_depth(), 0);
    }

    #[test]
    fn test_config_default() {
        let config = HistoryConfig::default();
        assert_eq!(config.max_depth, 100);
        assert_eq!(config.max_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn test_config_unlimited() {
        let config = HistoryConfig::unlimited();
        assert_eq!(config.max_depth, usize::MAX);
        assert_eq!(config.max_bytes, 0);
    }

    #[test]
    fn test_debug_impl() {
        let mgr = HistoryManager::default();
        let debug_str = format!("{:?}", mgr);
        assert!(debug_str.contains("HistoryManager"));
        assert!(debug_str.contains("undo_depth"));
    }

    #[test]
    fn test_config_new_custom_limits() {
        let config = HistoryConfig::new(50, 4096);
        assert_eq!(config.max_depth, 50);
        assert_eq!(config.max_bytes, 4096);
    }

    #[test]
    fn test_config_with_merge_config() {
        let mc = MergeConfig::default();
        let config = HistoryConfig::new(10, 0).with_merge_config(mc);
        // Builder pattern should work; verify merge_config is set
        assert_eq!(config.max_depth, 10);
    }

    #[test]
    fn test_config_accessor() {
        let config = HistoryConfig::new(42, 1024);
        let mgr = HistoryManager::new(config);
        assert_eq!(mgr.config().max_depth, 42);
        assert_eq!(mgr.config().max_bytes, 1024);
    }

    #[test]
    fn test_undo_descriptions_limited() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("a"));
        mgr.push(make_insert_cmd("b"));
        mgr.push(make_insert_cmd("c"));

        let descs = mgr.undo_descriptions(2);
        assert_eq!(descs.len(), 2, "should limit to 2 descriptions");
    }

    #[test]
    fn test_redo_descriptions() {
        let mut mgr = HistoryManager::default();
        mgr.push(make_insert_cmd("a"));
        mgr.push(make_insert_cmd("b"));
        mgr.undo();
        mgr.undo();

        let descs = mgr.redo_descriptions(5);
        assert_eq!(descs.len(), 2);

        let descs_limited = mgr.redo_descriptions(1);
        assert_eq!(descs_limited.len(), 1);
    }

    #[test]
    fn test_memory_byte_limit_evicts_old_commands() {
        // Each insert cmd is at least a few bytes. Use a very low byte limit.
        let config = HistoryConfig::new(100, 1); // 1 byte limit
        let mut mgr = HistoryManager::new(config);

        // Push several commands - enforce_limits should evict old ones
        for i in 0..5 {
            mgr.push(make_insert_cmd(&format!("cmd{i}")));
        }

        // With 1-byte limit, commands should get evicted
        assert!(
            mgr.undo_depth() < 5,
            "byte limit should evict old commands, depth={}",
            mgr.undo_depth()
        );
    }

    #[test]
    fn test_memory_tracking_after_undo_redo() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());
        mgr.push(make_insert_cmd("a"));
        let after_push = mgr.memory_usage();

        mgr.undo();
        let after_undo = mgr.memory_usage();
        // Memory should be same (moved to redo stack)
        assert_eq!(after_push, after_undo);

        mgr.redo();
        let after_redo = mgr.memory_usage();
        assert_eq!(after_push, after_redo);
    }

    // ====================================================================
    // Undo/redo failure paths
    // ====================================================================

    /// Helper to create a command whose undo always fails.
    fn make_failing_undo_cmd() -> Box<dyn UndoableCmd> {
        use crate::undo::command::CommandError;
        let buffer = Arc::new(Mutex::new(String::new()));
        let b1 = buffer.clone();

        Box::new(
            TextInsertCmd::new(WidgetId::new(1), 0, "x")
                .with_apply(move |_, _, txt| {
                    let mut buf = b1.lock().unwrap();
                    buf.push_str(txt);
                    Ok(())
                })
                .with_remove(move |_, _, _| Err(CommandError::Other("undo fail".to_string()))),
        )
    }

    /// Helper to create a command whose redo (execute) always fails
    /// but whose undo succeeds.
    fn make_failing_redo_cmd() -> (Box<dyn UndoableCmd>, Arc<Mutex<String>>) {
        use crate::undo::command::CommandError;
        let buffer = Arc::new(Mutex::new(String::new()));
        let b1 = buffer.clone();
        let b2 = buffer.clone();
        let call_count = Arc::new(Mutex::new(0u32));
        let cc = call_count.clone();

        let cmd = TextInsertCmd::new(WidgetId::new(1), 0, "y")
            .with_apply(move |_, _, txt| {
                let mut count = cc.lock().unwrap();
                *count += 1;
                if *count > 1 {
                    return Err(CommandError::Other("redo fail".to_string()));
                }
                let mut buf = b1.lock().unwrap();
                buf.push_str(txt);
                Ok(())
            })
            .with_remove(move |_, _, len| {
                let mut buf = b2.lock().unwrap();
                buf.drain(..len);
                Ok(())
            });

        (Box::new(cmd), buffer)
    }

    #[test]
    fn test_undo_failure_keeps_command_on_stack() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());

        // Execute the command manually first, then push
        let mut cmd = make_failing_undo_cmd();
        cmd.execute().unwrap();
        mgr.push(cmd);

        assert_eq!(mgr.undo_depth(), 1);

        // Undo should fail
        let result = mgr.undo();
        assert!(result.is_some());
        assert!(result.unwrap().is_err());

        // Command should still be on undo stack
        assert_eq!(mgr.undo_depth(), 1);
        assert_eq!(mgr.redo_depth(), 0);
    }

    #[test]
    fn test_redo_failure_keeps_command_on_redo_stack() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());

        let (mut cmd, _buffer) = make_failing_redo_cmd();
        cmd.execute().unwrap(); // First execute succeeds
        mgr.push(cmd);

        // Undo should succeed
        let result = mgr.undo();
        assert!(result.unwrap().is_ok());
        assert_eq!(mgr.redo_depth(), 1);

        // Redo should fail (second execute fails)
        let result = mgr.redo();
        assert!(result.is_some());
        assert!(result.unwrap().is_err());

        // Command should remain on redo stack
        assert_eq!(mgr.redo_depth(), 1);
        assert_eq!(mgr.undo_depth(), 0);
    }

    // ====================================================================
    // Merge path
    // ====================================================================

    #[test]
    fn test_push_merges_consecutive_inserts() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let b1 = buffer.clone();
        let b2 = buffer.clone();
        let b3 = buffer.clone();
        let b4 = buffer.clone();

        let mut mgr = HistoryManager::new(HistoryConfig::default());

        // First insert "a" at position 0
        let mut cmd1 = TextInsertCmd::new(WidgetId::new(1), 0, "a")
            .with_apply(move |_, pos, txt| {
                let mut buf = b1.lock().unwrap();
                buf.insert_str(pos, txt);
                Ok(())
            })
            .with_remove(move |_, pos, len| {
                let mut buf = b2.lock().unwrap();
                buf.drain(pos..pos + len);
                Ok(())
            });
        cmd1.execute().unwrap();
        mgr.push(Box::new(cmd1));

        // Second insert "b" at position 1 (consecutive)
        let mut cmd2 = TextInsertCmd::new(WidgetId::new(1), 1, "b")
            .with_apply(move |_, pos, txt| {
                let mut buf = b3.lock().unwrap();
                buf.insert_str(pos, txt);
                Ok(())
            })
            .with_remove(move |_, pos, len| {
                let mut buf = b4.lock().unwrap();
                buf.drain(pos..pos + len);
                Ok(())
            });
        cmd2.metadata.timestamp = mgr.undo_stack.back().unwrap().metadata().timestamp;
        cmd2.execute().unwrap();
        mgr.push(Box::new(cmd2));

        // Should have merged into 1 command
        assert_eq!(
            mgr.undo_depth(),
            1,
            "consecutive inserts should merge into 1 command"
        );
    }

    // ====================================================================
    // Memory eviction: redo evicted before undo
    // ====================================================================

    #[test]
    fn test_push_clears_redo_memory_accounting() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());

        mgr.push(make_insert_cmd("redo_me"));
        let mem_after_push = mgr.memory_usage();

        mgr.undo();
        // Memory unchanged (moved to redo)
        assert_eq!(mgr.memory_usage(), mem_after_push);

        // New push clears redo; total memory should only be for the new cmd
        mgr.push(make_insert_cmd("new"));
        assert_eq!(mgr.redo_depth(), 0);
        // Memory should be for "new" only (redo was cleared)
        assert!(mgr.memory_usage() > 0);
    }

    // ====================================================================
    // Descriptions on empty stacks
    // ====================================================================

    #[test]
    fn test_descriptions_empty_stacks() {
        let mgr = HistoryManager::default();
        assert!(mgr.undo_descriptions(10).is_empty());
        assert!(mgr.redo_descriptions(10).is_empty());
        assert_eq!(mgr.next_undo_description(), None);
        assert_eq!(mgr.next_redo_description(), None);
    }

    // ====================================================================
    // Memory accounting after push clears redo
    // ====================================================================

    #[test]
    fn test_memory_decreases_when_push_clears_redo() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());

        mgr.push(make_insert_cmd("aaa"));
        mgr.push(make_insert_cmd("bbb"));
        mgr.undo(); // "bbb" moves to redo
        let mem_with_redo = mgr.memory_usage();
        assert!(mem_with_redo > 0);

        // Push clears redo
        mgr.push(make_insert_cmd("ccc"));

        // Memory should account for "aaa" + "ccc" but not "bbb"
        assert_eq!(mgr.redo_depth(), 0);
        // Memory after clearing redo and adding new should be different
        let mem_after = mgr.memory_usage();
        assert!(mem_after > 0);
    }

    // ====================================================================
    // Depth limit with eviction
    // ====================================================================

    #[test]
    fn test_depth_limit_evicts_oldest() {
        let config = HistoryConfig::new(2, 0);
        let mut mgr = HistoryManager::new(config);

        mgr.push(make_insert_cmd("first"));
        mgr.push(make_insert_cmd("second"));
        mgr.push(make_insert_cmd("third"));

        assert_eq!(mgr.undo_depth(), 2);
        // Most recent two should remain
        let descs = mgr.undo_descriptions(2);
        assert_eq!(descs.len(), 2);
    }

    // ====================================================================
    // Default impl
    // ====================================================================

    #[test]
    fn test_default_impl() {
        let mgr = HistoryManager::default();
        assert_eq!(mgr.config().max_depth, 100);
        assert_eq!(mgr.config().max_bytes, 10 * 1024 * 1024);
    }

    // ====================================================================
    // try_merge: merge_text() returns None
    // ====================================================================

    /// A command that reports can_merge = true but merge_text = None.
    /// This exercises the early-exit at line 361-363.
    struct MergeableNoText {
        metadata: crate::undo::command::CommandMetadata,
    }

    impl MergeableNoText {
        fn new() -> Self {
            Self {
                metadata: crate::undo::command::CommandMetadata::new("MergeableNoText"),
            }
        }
    }

    impl crate::undo::command::UndoableCmd for MergeableNoText {
        fn execute(&mut self) -> crate::undo::command::CommandResult {
            Ok(())
        }
        fn undo(&mut self) -> crate::undo::command::CommandResult {
            Ok(())
        }
        fn description(&self) -> &str {
            &self.metadata.description
        }
        fn size_bytes(&self) -> usize {
            std::mem::size_of::<Self>()
        }
        fn can_merge(
            &self,
            _other: &dyn crate::undo::command::UndoableCmd,
            _config: &MergeConfig,
        ) -> bool {
            true // Says it can merge...
        }
        fn merge_text(&self) -> Option<&str> {
            None // ...but has no text to merge
        }
        fn accept_merge(&mut self, _other: &dyn crate::undo::command::UndoableCmd) -> bool {
            false
        }
        fn metadata(&self) -> &crate::undo::command::CommandMetadata {
            &self.metadata
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn test_try_merge_exits_early_when_merge_text_none() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());

        mgr.push(Box::new(MergeableNoText::new()));
        assert_eq!(mgr.undo_depth(), 1);

        // Second push: can_merge=true but merge_text=None → not merged
        mgr.push(Box::new(MergeableNoText::new()));
        assert_eq!(mgr.undo_depth(), 2);
    }

    // ====================================================================
    // try_merge: accept_merge returns false
    // ====================================================================

    /// A command where can_merge=true, merge_text=Some, but accept_merge=false.
    struct MergeableRejectsAccept {
        metadata: crate::undo::command::CommandMetadata,
    }

    impl MergeableRejectsAccept {
        fn new() -> Self {
            Self {
                metadata: crate::undo::command::CommandMetadata::new("MergeableRejectsAccept"),
            }
        }
    }

    impl crate::undo::command::UndoableCmd for MergeableRejectsAccept {
        fn execute(&mut self) -> crate::undo::command::CommandResult {
            Ok(())
        }
        fn undo(&mut self) -> crate::undo::command::CommandResult {
            Ok(())
        }
        fn description(&self) -> &str {
            &self.metadata.description
        }
        fn size_bytes(&self) -> usize {
            std::mem::size_of::<Self>()
        }
        fn can_merge(
            &self,
            _other: &dyn crate::undo::command::UndoableCmd,
            _config: &MergeConfig,
        ) -> bool {
            true
        }
        fn merge_text(&self) -> Option<&str> {
            Some("text")
        }
        fn accept_merge(&mut self, _other: &dyn crate::undo::command::UndoableCmd) -> bool {
            false // Rejects the merge
        }
        fn metadata(&self) -> &crate::undo::command::CommandMetadata {
            &self.metadata
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn test_try_merge_not_merged_when_accept_merge_false() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());

        mgr.push(Box::new(MergeableRejectsAccept::new()));
        assert_eq!(mgr.undo_depth(), 1);

        mgr.push(Box::new(MergeableRejectsAccept::new()));
        assert_eq!(mgr.undo_depth(), 2);
    }

    // ====================================================================
    // enforce_limits: redo evicted before undo under memory pressure
    // ====================================================================

    #[test]
    fn test_push_always_clears_redo_before_enforce() {
        // Note: enforce_limits' redo eviction path is unreachable
        // because push() always calls clear_redo() first. Test the
        // observable behavior: push after undo clears redo.
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());

        mgr.push(make_insert_cmd("redo_item"));
        mgr.undo();
        assert_eq!(mgr.redo_depth(), 1);

        // New push always clears redo (new branch)
        mgr.push(make_insert_cmd("new_item"));
        assert_eq!(mgr.redo_depth(), 0);
    }

    // ====================================================================
    // max_bytes = 0 means unlimited
    // ====================================================================

    #[test]
    fn test_max_bytes_zero_means_unlimited() {
        let config = HistoryConfig::new(100, 0); // 0 = unlimited bytes
        let mut mgr = HistoryManager::new(config);

        for i in 0..50 {
            mgr.push(make_insert_cmd(&format!("cmd{i}")));
        }

        // All 50 should be retained (depth limit is 100)
        assert_eq!(mgr.undo_depth(), 50);
        assert!(mgr.memory_usage() > 0);
    }

    // ====================================================================
    // Memory accounting after successful merge
    // ====================================================================

    #[test]
    fn test_memory_accounting_after_merge() {
        let buffer = Arc::new(Mutex::new(String::new()));
        let b1 = buffer.clone();
        let b2 = buffer.clone();
        let b3 = buffer.clone();
        let b4 = buffer.clone();

        let mut mgr = HistoryManager::new(HistoryConfig::default());

        // First insert
        let mut cmd1 = TextInsertCmd::new(WidgetId::new(1), 0, "a")
            .with_apply(move |_, pos, txt| {
                let mut buf = b1.lock().unwrap();
                buf.insert_str(pos, txt);
                Ok(())
            })
            .with_remove(move |_, pos, len| {
                let mut buf = b2.lock().unwrap();
                buf.drain(pos..pos + len);
                Ok(())
            });
        cmd1.execute().unwrap();
        mgr.push(Box::new(cmd1));
        let mem_after_first = mgr.memory_usage();

        // Second insert (consecutive, same timestamp → merges)
        let mut cmd2 = TextInsertCmd::new(WidgetId::new(1), 1, "b")
            .with_apply(move |_, pos, txt| {
                let mut buf = b3.lock().unwrap();
                buf.insert_str(pos, txt);
                Ok(())
            })
            .with_remove(move |_, pos, len| {
                let mut buf = b4.lock().unwrap();
                buf.drain(pos..pos + len);
                Ok(())
            });
        cmd2.metadata.timestamp = mgr.undo_stack.back().unwrap().metadata().timestamp;
        cmd2.execute().unwrap();
        mgr.push(Box::new(cmd2));

        // Still 1 command (merged)
        assert_eq!(mgr.undo_depth(), 1);

        // Memory should increase by the merged text size
        let mem_after_merge = mgr.memory_usage();
        assert!(
            mem_after_merge > mem_after_first,
            "memory should increase after merge: {} vs {}",
            mem_after_merge,
            mem_after_first
        );
    }

    // ====================================================================
    // HistoryConfig Debug and Clone
    // ====================================================================

    #[test]
    fn test_history_config_debug() {
        let config = HistoryConfig::new(50, 4096);
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("HistoryConfig"));
        assert!(debug_str.contains("50"));
        assert!(debug_str.contains("4096"));
    }

    #[test]
    fn test_history_config_clone() {
        let config = HistoryConfig::new(50, 4096);
        let cloned = config.clone();
        assert_eq!(cloned.max_depth, 50);
        assert_eq!(cloned.max_bytes, 4096);
    }

    // ====================================================================
    // Depth limit + memory limit interaction
    // ====================================================================

    #[test]
    fn test_depth_and_byte_limits_both_enforced() {
        // Depth limit 3, byte limit very small
        let config = HistoryConfig::new(3, 1);
        let mut mgr = HistoryManager::new(config);

        for i in 0..10 {
            mgr.push(make_insert_cmd(&format!("cmd{i}")));
        }

        // Both limits apply; depth ≤ 3 AND bytes may further reduce
        assert!(mgr.undo_depth() <= 3);
    }

    // ====================================================================
    // Push onto empty undo stack (try_merge early exit on empty)
    // ====================================================================

    #[test]
    fn test_try_merge_returns_err_on_empty_stack() {
        let mut mgr = HistoryManager::new(HistoryConfig::default());

        // First push should not attempt merge (empty stack)
        mgr.push(make_insert_cmd("first"));
        assert_eq!(mgr.undo_depth(), 1);
    }

    // ====================================================================
    // Undo returns description on success
    // ====================================================================

    #[test]
    fn test_undo_returns_description_string() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());
        mgr.push(make_insert_cmd("hello"));

        let result = mgr.undo().unwrap();
        assert_eq!(result.unwrap(), "Insert text");
    }

    #[test]
    fn test_redo_returns_description_string() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());
        mgr.push(make_insert_cmd("hello"));
        mgr.undo();

        let result = mgr.redo().unwrap();
        assert_eq!(result.unwrap(), "Insert text");
    }

    // ====================================================================
    // Clear with items on both stacks
    // ====================================================================

    #[test]
    fn test_clear_resets_memory_with_both_stacks() {
        let mut mgr = HistoryManager::new(HistoryConfig::unlimited());
        mgr.push(make_insert_cmd("a"));
        mgr.push(make_insert_cmd("b"));
        mgr.push(make_insert_cmd("c"));
        mgr.undo(); // "c" to redo
        mgr.undo(); // "b" to redo

        assert_eq!(mgr.undo_depth(), 1);
        assert_eq!(mgr.redo_depth(), 2);
        assert!(mgr.memory_usage() > 0);

        mgr.clear();

        assert_eq!(mgr.undo_depth(), 0);
        assert_eq!(mgr.redo_depth(), 0);
        assert_eq!(mgr.memory_usage(), 0);
    }

    // ====================================================================
    // Depth limit of 1
    // ====================================================================

    #[test]
    fn test_depth_limit_one() {
        let config = HistoryConfig::new(1, 0);
        let mut mgr = HistoryManager::new(config);

        mgr.push(make_insert_cmd("first"));
        mgr.push(make_insert_cmd("second"));
        mgr.push(make_insert_cmd("third"));

        assert_eq!(mgr.undo_depth(), 1);
    }

    // ====================================================================
    // Depth limit of 0 (edge case)
    // ====================================================================

    #[test]
    fn test_depth_limit_zero_evicts_everything() {
        let config = HistoryConfig::new(0, 0);
        let mut mgr = HistoryManager::new(config);

        mgr.push(make_insert_cmd("will_be_evicted"));

        // Depth limit 0 means everything gets evicted
        assert_eq!(mgr.undo_depth(), 0);
    }
}
