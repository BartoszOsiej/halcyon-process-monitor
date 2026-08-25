#![forbid(unsafe_code)]

//! Undo support for widgets.
//!
//! This module provides the [`UndoSupport`] trait that widgets can implement
//! to enable undo/redo functionality for their state changes.
//!
//! # Design
//!
//! The undo system is based on the Command Pattern. Each undoable operation
//! creates a command that knows how to:
//! 1. Execute the operation (already done when the command is created)
//! 2. Undo the operation (reverse the change)
//! 3. Redo the operation (reapply the change)
//!
//! Commands are stored in a history stack managed by [`HistoryManager`].
//!
//! # Usage
//!
//! Widgets that implement `UndoSupport` can generate commands for their
//! state changes. These commands can then be pushed to a history manager
//! for undo/redo support.
//!
//! ```ignore
//! use ftui_widgets::undo_support::{UndoSupport, TextEditOperation};
//! use ftui_runtime::undo::HistoryManager;
//!
//! let mut history = HistoryManager::default();
//! let mut input = TextInput::new();
//!
//! // Perform an edit and create an undo command
//! if let Some(cmd) = input.create_undo_command(TextEditOperation::Insert {
//!     position: 0,
//!     text: "Hello".to_string(),
//! }) {
//!     history.push(cmd);
//! }
//! ```
//!
//! [`HistoryManager`]: ftui_runtime::undo::HistoryManager

use std::any::Any;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Unique identifier for a widget instance.
///
/// Used to associate undo commands with specific widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UndoWidgetId(u64);

impl UndoWidgetId {
    /// Create a new unique widget ID.
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Create a widget ID from a raw value.
    ///
    /// Use this when you need to associate commands with a specific widget.
    #[must_use]
    pub const fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl Default for UndoWidgetId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for UndoWidgetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Widget({})", self.0)
    }
}

/// Text edit operation types.
///
/// These represent the atomic operations that can be performed on text.
#[derive(Debug, Clone)]
pub enum TextEditOperation {
    /// Insert text at a position.
    Insert {
        /// Grapheme index where text was inserted.
        position: usize,
        /// The inserted text.
        text: String,
    },
    /// Delete text at a position.
    Delete {
        /// Grapheme index where deletion started.
        position: usize,
        /// The deleted text (for undo).
        deleted_text: String,
    },
    /// Replace text at a position.
    Replace {
        /// Grapheme index where replacement started.
        position: usize,
        /// The old text (for undo).
        old_text: String,
        /// The new text.
        new_text: String,
    },
    /// Set the entire value.
    SetValue {
        /// The old value (for undo).
        old_value: String,
        /// The new value.
        new_value: String,
    },
}

impl TextEditOperation {
    /// Get a description of this operation.
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Insert { .. } => "Insert text",
            Self::Delete { .. } => "Delete text",
            Self::Replace { .. } => "Replace text",
            Self::SetValue { .. } => "Set value",
        }
    }

    /// Calculate the size in bytes of this operation.
    #[must_use]
    pub fn size_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Insert { text, .. } => text.len(),
                Self::Delete { deleted_text, .. } => deleted_text.len(),
                Self::Replace {
                    old_text, new_text, ..
                } => old_text.len() + new_text.len(),
                Self::SetValue {
                    old_value,
                    new_value,
                } => old_value.len() + new_value.len(),
            }
    }
}

/// Selection state operation types.
#[derive(Debug, Clone)]
pub enum SelectionOperation {
    /// Selection changed.
    Changed {
        /// Old selection anchor.
        old_anchor: Option<usize>,
        /// Old cursor position.
        old_cursor: usize,
        /// New selection anchor.
        new_anchor: Option<usize>,
        /// New cursor position.
        new_cursor: usize,
    },
}

/// Tree expansion operation types.
#[derive(Debug, Clone)]
pub enum TreeOperation {
    /// Node expanded.
    Expand {
        /// Path to the node (indices).
        path: Vec<usize>,
    },
    /// Node collapsed.
    Collapse {
        /// Path to the node (indices).
        path: Vec<usize>,
    },
    /// Multiple nodes toggled.
    ToggleBatch {
        /// Paths that were expanded.
        expanded: Vec<Vec<usize>>,
        /// Paths that were collapsed.
        collapsed: Vec<Vec<usize>>,
    },
}

impl TreeOperation {
    /// Get a description of this operation.
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Expand { .. } => "Expand node",
            Self::Collapse { .. } => "Collapse node",
            Self::ToggleBatch { .. } => "Toggle nodes",
        }
    }
}

/// List selection operation types.
#[derive(Debug, Clone)]
pub enum ListOperation {
    /// Selection changed.
    Select {
        /// Old selection.
        old_selection: Option<usize>,
        /// New selection.
        new_selection: Option<usize>,
    },
    /// Multiple selection changed.
    MultiSelect {
        /// Old selections.
        old_selections: Vec<usize>,
        /// New selections.
        new_selections: Vec<usize>,
    },
}

impl ListOperation {
    /// Get a description of this operation.
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Select { .. } => "Change selection",
            Self::MultiSelect { .. } => "Change selections",
        }
    }
}

/// Table operation types.
#[derive(Debug, Clone)]
pub enum TableOperation {
    /// Sort column changed.
    Sort {
        /// Old sort column.
        old_column: Option<usize>,
        /// Old sort ascending.
        old_ascending: bool,
        /// New sort column.
        new_column: Option<usize>,
        /// New sort ascending.
        new_ascending: bool,
    },
    /// Filter applied.
    Filter {
        /// Old filter string.
        old_filter: String,
        /// New filter string.
        new_filter: String,
    },
    /// Row selection changed.
    SelectRow {
        /// Old selected row.
        old_row: Option<usize>,
        /// New selected row.
        new_row: Option<usize>,
    },
}

impl TableOperation {
    /// Get a description of this operation.
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Sort { .. } => "Change sort",
            Self::Filter { .. } => "Apply filter",
            Self::SelectRow { .. } => "Select row",
        }
    }
}

/// Callback for applying a text edit operation.
pub type TextEditApplyFn =
    Box<dyn Fn(UndoWidgetId, &TextEditOperation) -> Result<(), String> + Send + Sync>;

/// Callback for undoing a text edit operation.
pub type TextEditUndoFn =
    Box<dyn Fn(UndoWidgetId, &TextEditOperation) -> Result<(), String> + Send + Sync>;

/// A widget undo command for text editing.
pub struct WidgetTextEditCmd {
    /// Widget ID this command operates on.
    widget_id: UndoWidgetId,
    /// The operation.
    operation: TextEditOperation,
    /// Apply callback.
    apply_fn: Option<TextEditApplyFn>,
    /// Undo callback.
    undo_fn: Option<TextEditUndoFn>,
    /// Whether the operation has been executed.
    executed: bool,
}

impl WidgetTextEditCmd {
    /// Create a new text edit command.
    #[must_use]
    pub fn new(widget_id: UndoWidgetId, operation: TextEditOperation) -> Self {
        Self {
            widget_id,
            operation,
            apply_fn: None,
            undo_fn: None,
            executed: false,
        }
    }

    /// Set the apply callback (builder).
    #[must_use]
    pub fn with_apply<F>(mut self, f: F) -> Self
    where
        F: Fn(UndoWidgetId, &TextEditOperation) -> Result<(), String> + Send + Sync + 'static,
    {
        self.apply_fn = Some(Box::new(f));
        self
    }

    /// Set the undo callback (builder).
    #[must_use]
    pub fn with_undo<F>(mut self, f: F) -> Self
    where
        F: Fn(UndoWidgetId, &TextEditOperation) -> Result<(), String> + Send + Sync + 'static,
    {
        self.undo_fn = Some(Box::new(f));
        self
    }

    /// Get the widget ID.
    #[must_use]
    pub fn widget_id(&self) -> UndoWidgetId {
        self.widget_id
    }

    /// Get the operation.
    #[must_use]
    pub fn operation(&self) -> &TextEditOperation {
        &self.operation
    }
}

impl fmt::Debug for WidgetTextEditCmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WidgetTextEditCmd")
            .field("widget_id", &self.widget_id)
            .field("operation", &self.operation)
            .field("executed", &self.executed)
            .finish()
    }
}

// Implement UndoableCmd trait from ftui_runtime
// Note: We can't directly implement the trait here because it's in ftui_runtime
// and we can't have a circular dependency. Instead, we provide methods that
// match the trait's interface, and the integration happens at runtime.

impl WidgetTextEditCmd {
    /// Execute the command.
    #[must_use = "handle the result; errors indicate the edit was not applied"]
    pub fn execute(&mut self) -> Result<(), String> {
        if let Some(ref apply_fn) = self.apply_fn {
            apply_fn(self.widget_id, &self.operation)?;
        }
        self.executed = true;
        Ok(())
    }

    /// Undo the command.
    #[must_use = "handle the result; errors indicate the undo was not applied"]
    pub fn undo(&mut self) -> Result<(), String> {
        if let Some(ref undo_fn) = self.undo_fn {
            undo_fn(self.widget_id, &self.operation)?;
        }
        self.executed = false;
        Ok(())
    }

    /// Redo the command (same as execute).
    #[must_use = "handle the result; errors indicate the redo was not applied"]
    pub fn redo(&mut self) -> Result<(), String> {
        self.execute()
    }

    /// Get the description.
    #[must_use]
    pub fn description(&self) -> &'static str {
        self.operation.description()
    }

    /// Get the size in bytes.
    #[must_use]
    pub fn size_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.operation.size_bytes()
    }
}

/// Trait for widgets that support undo operations.
///
/// Widgets implement this trait to provide undo/redo functionality.
/// The trait provides a standardized way to:
/// 1. Track widget identity for command association
/// 2. Create undo commands for state changes
/// 3. Restore state from undo/redo operations
pub trait UndoSupport {
    /// Get the widget's unique ID for undo tracking.
    fn undo_widget_id(&self) -> UndoWidgetId;

    /// Create a snapshot of the current state for undo purposes.
    ///
    /// This is used to create "before" state for operations.
    fn create_snapshot(&self) -> Box<dyn Any + Send>;

    /// Restore state from a snapshot.
    ///
    /// Returns true if the restore was successful.
    fn restore_snapshot(&mut self, snapshot: &dyn Any) -> bool;
}

/// Extension trait for text input widgets with undo support.
pub trait TextInputUndoExt: UndoSupport {
    /// Get the current text value.
    fn text_value(&self) -> &str;

    /// Set the text value directly (for undo/redo).
    fn set_text_value(&mut self, value: &str);

    /// Get the current cursor position.
    fn cursor_position(&self) -> usize;

    /// Set the cursor position directly.
    fn set_cursor_position(&mut self, pos: usize);

    /// Insert text at a position.
    fn insert_text_at(&mut self, position: usize, text: &str);

    /// Delete text at a range.
    fn delete_text_range(&mut self, start: usize, end: usize);
}

/// Extension trait for tree widgets with undo support.
pub trait TreeUndoExt: UndoSupport {
    /// Check if a node is expanded.
    fn is_node_expanded(&self, path: &[usize]) -> bool;

    /// Expand a node.
    fn expand_node(&mut self, path: &[usize]);

    /// Collapse a node.
    fn collapse_node(&mut self, path: &[usize]);
}

/// Extension trait for list widgets with undo support.
pub trait ListUndoExt: UndoSupport {
    /// Get the current selection.
    fn selected_index(&self) -> Option<usize>;

    /// Set the selection.
    fn set_selected_index(&mut self, index: Option<usize>);
}

/// Extension trait for table widgets with undo support.
pub trait TableUndoExt: UndoSupport {
    /// Get the current sort state.
    fn sort_state(&self) -> (Option<usize>, bool);

    /// Set the sort state.
    fn set_sort_state(&mut self, column: Option<usize>, ascending: bool);

    /// Get the current filter.
    fn filter_text(&self) -> &str;

    /// Set the filter.
    fn set_filter_text(&mut self, filter: &str);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_widget_id_uniqueness() {
        let id1 = UndoWidgetId::new();
        let id2 = UndoWidgetId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_undo_widget_id_from_raw() {
        let id = UndoWidgetId::from_raw(42);
        assert_eq!(id.raw(), 42);
    }

    #[test]
    fn test_text_edit_operation_description() {
        assert_eq!(
            TextEditOperation::Insert {
                position: 0,
                text: "x".to_string()
            }
            .description(),
            "Insert text"
        );
        assert_eq!(
            TextEditOperation::Delete {
                position: 0,
                deleted_text: "x".to_string()
            }
            .description(),
            "Delete text"
        );
    }

    #[test]
    fn test_text_edit_operation_size_bytes() {
        let op = TextEditOperation::Insert {
            position: 0,
            text: "hello".to_string(),
        };
        assert!(op.size_bytes() > 5);
    }

    #[test]
    fn test_widget_text_edit_cmd_creation() {
        let widget_id = UndoWidgetId::new();
        let cmd = WidgetTextEditCmd::new(
            widget_id,
            TextEditOperation::Insert {
                position: 0,
                text: "test".to_string(),
            },
        );
        assert_eq!(cmd.widget_id(), widget_id);
        assert_eq!(cmd.description(), "Insert text");
    }

    #[test]
    fn test_widget_text_edit_cmd_with_callbacks() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let applied = Arc::new(AtomicBool::new(false));
        let undone = Arc::new(AtomicBool::new(false));
        let applied_clone = applied.clone();
        let undone_clone = undone.clone();

        let widget_id = UndoWidgetId::new();
        let mut cmd = WidgetTextEditCmd::new(
            widget_id,
            TextEditOperation::Insert {
                position: 0,
                text: "test".to_string(),
            },
        )
        .with_apply(move |_, _| {
            applied_clone.store(true, Ordering::SeqCst);
            Ok(())
        })
        .with_undo(move |_, _| {
            undone_clone.store(true, Ordering::SeqCst);
            Ok(())
        });

        cmd.execute().unwrap();
        assert!(applied.load(Ordering::SeqCst));

        cmd.undo().unwrap();
        assert!(undone.load(Ordering::SeqCst));
    }

    #[test]
    fn test_tree_operation_description() {
        assert_eq!(
            TreeOperation::Expand { path: vec![0] }.description(),
            "Expand node"
        );
        assert_eq!(
            TreeOperation::Collapse { path: vec![0] }.description(),
            "Collapse node"
        );
    }

    #[test]
    fn test_list_operation_description() {
        assert_eq!(
            ListOperation::Select {
                old_selection: None,
                new_selection: Some(0)
            }
            .description(),
            "Change selection"
        );
    }

    #[test]
    fn test_table_operation_description() {
        assert_eq!(
            TableOperation::Sort {
                old_column: None,
                old_ascending: true,
                new_column: Some(0),
                new_ascending: true
            }
            .description(),
            "Change sort"
        );
    }

    // --- UndoWidgetId ---

    #[test]
    fn widget_id_display() {
        let id = UndoWidgetId::from_raw(7);
        assert_eq!(format!("{id}"), "Widget(7)");
    }

    #[test]
    fn widget_id_default_is_unique() {
        let a = UndoWidgetId::default();
        let b = UndoWidgetId::default();
        assert_ne!(a, b);
    }

    #[test]
    fn widget_id_hash_eq() {
        let id = UndoWidgetId::from_raw(99);
        let id2 = UndoWidgetId::from_raw(99);
        assert_eq!(id, id2);

        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(id);
        assert!(set.contains(&id2));
    }

    // --- TextEditOperation descriptions and size_bytes ---

    #[test]
    fn text_edit_replace_description() {
        let op = TextEditOperation::Replace {
            position: 0,
            old_text: "old".to_string(),
            new_text: "new".to_string(),
        };
        assert_eq!(op.description(), "Replace text");
    }

    #[test]
    fn text_edit_set_value_description() {
        let op = TextEditOperation::SetValue {
            old_value: "".to_string(),
            new_value: "hello".to_string(),
        };
        assert_eq!(op.description(), "Set value");
    }

    #[test]
    fn text_edit_delete_size_bytes() {
        let op = TextEditOperation::Delete {
            position: 5,
            deleted_text: "abc".to_string(),
        };
        assert!(op.size_bytes() >= 3);
    }

    #[test]
    fn text_edit_replace_size_bytes() {
        let op = TextEditOperation::Replace {
            position: 0,
            old_text: "aaa".to_string(),
            new_text: "bbbbb".to_string(),
        };
        // Should include both old and new text lengths
        assert!(op.size_bytes() >= 8); // 3 + 5
    }

    #[test]
    fn text_edit_set_value_size_bytes() {
        let op = TextEditOperation::SetValue {
            old_value: "x".to_string(),
            new_value: "yyyy".to_string(),
        };
        assert!(op.size_bytes() >= 5); // 1 + 4
    }

    // --- WidgetTextEditCmd ---

    #[test]
    fn cmd_execute_without_callbacks_succeeds() {
        let mut cmd = WidgetTextEditCmd::new(
            UndoWidgetId::from_raw(1),
            TextEditOperation::Insert {
                position: 0,
                text: "hi".to_string(),
            },
        );
        assert!(cmd.execute().is_ok());
    }

    #[test]
    fn cmd_undo_without_callbacks_succeeds() {
        let mut cmd = WidgetTextEditCmd::new(
            UndoWidgetId::from_raw(1),
            TextEditOperation::Delete {
                position: 0,
                deleted_text: "x".to_string(),
            },
        );
        assert!(cmd.undo().is_ok());
    }

    #[test]
    fn cmd_redo_calls_execute() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();

        let mut cmd = WidgetTextEditCmd::new(
            UndoWidgetId::from_raw(1),
            TextEditOperation::Insert {
                position: 0,
                text: "t".to_string(),
            },
        )
        .with_apply(move |_, _| {
            count_clone.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        cmd.execute().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);

        cmd.redo().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cmd_debug_format() {
        let cmd = WidgetTextEditCmd::new(
            UndoWidgetId::from_raw(5),
            TextEditOperation::Insert {
                position: 0,
                text: "abc".to_string(),
            },
        );
        let dbg = format!("{cmd:?}");
        assert!(dbg.contains("WidgetTextEditCmd"));
        assert!(dbg.contains("Insert"));
    }

    #[test]
    fn cmd_size_bytes_nonzero() {
        let cmd = WidgetTextEditCmd::new(
            UndoWidgetId::from_raw(1),
            TextEditOperation::Insert {
                position: 0,
                text: "hello world".to_string(),
            },
        );
        assert!(cmd.size_bytes() > 11);
    }

    // --- TreeOperation ---

    #[test]
    fn tree_toggle_batch_description() {
        let op = TreeOperation::ToggleBatch {
            expanded: vec![vec![0, 1]],
            collapsed: vec![vec![2]],
        };
        assert_eq!(op.description(), "Toggle nodes");
    }

    // --- ListOperation ---

    #[test]
    fn list_multi_select_description() {
        let op = ListOperation::MultiSelect {
            old_selections: vec![0, 1],
            new_selections: vec![2, 3],
        };
        assert_eq!(op.description(), "Change selections");
    }

    // --- TableOperation ---

    #[test]
    fn table_filter_description() {
        let op = TableOperation::Filter {
            old_filter: "".to_string(),
            new_filter: "test".to_string(),
        };
        assert_eq!(op.description(), "Apply filter");
    }

    #[test]
    fn table_select_row_description() {
        let op = TableOperation::SelectRow {
            old_row: Some(0),
            new_row: Some(5),
        };
        assert_eq!(op.description(), "Select row");
    }

    // --- SelectionOperation ---

    #[test]
    fn selection_operation_fields() {
        let op = SelectionOperation::Changed {
            old_anchor: Some(0),
            old_cursor: 5,
            new_anchor: None,
            new_cursor: 10,
        };
        let dbg = format!("{op:?}");
        assert!(dbg.contains("Changed"));
    }
}
