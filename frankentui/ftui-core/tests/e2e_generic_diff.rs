//! E2E integration tests for Datatype-Generic Diff/Patch (bd-3jak8.4)
//!
//! Tests the round-trip property `patch(old, diff(old, new)) == new` on
//! representative widget-state-like types, verifies compression benefit,
//! and emits structured JSONL evidence.

use ftui_core::generic_diff::*;
use ftui_core::generic_repr::*;
use std::time::Instant;

// ── Representative widget state types ────────────────────────────

/// Simulates ListState: selected index + offset + item count.
#[derive(Clone, Debug, PartialEq)]
struct ListState {
    selected: usize,
    offset: usize,
    item_count: usize,
}

impl GenericRepr for ListState {
    type Repr = Product<Field<usize>, Product<Field<usize>, Product<Field<usize>, Unit>>>;
    fn into_repr(self) -> Self::Repr {
        Product(
            Field::new("selected", self.selected),
            Product(
                Field::new("offset", self.offset),
                Product(Field::new("item_count", self.item_count), Unit),
            ),
        )
    }
    fn from_repr(repr: Self::Repr) -> Self {
        Self {
            selected: repr.0.value,
            offset: repr.1.0.value,
            item_count: repr.1.1.0.value,
        }
    }
}

/// Simulates TableState: selected row/col + sort column + sort direction.
#[derive(Clone, Debug, PartialEq)]
struct TableState {
    row: usize,
    col: usize,
    sort_col: usize,
    sort_asc: bool,
}

impl GenericRepr for TableState {
    type Repr = Product<
        Field<usize>,
        Product<Field<usize>, Product<Field<usize>, Product<Field<bool>, Unit>>>,
    >;
    fn into_repr(self) -> Self::Repr {
        Product(
            Field::new("row", self.row),
            Product(
                Field::new("col", self.col),
                Product(
                    Field::new("sort_col", self.sort_col),
                    Product(Field::new("sort_asc", self.sort_asc), Unit),
                ),
            ),
        )
    }
    fn from_repr(repr: Self::Repr) -> Self {
        Self {
            row: repr.0.value,
            col: repr.1.0.value,
            sort_col: repr.1.1.0.value,
            sort_asc: repr.1.1.1.0.value,
        }
    }
}

/// Simulates TabsState: active tab index.
#[derive(Clone, Debug, PartialEq)]
struct TabsState {
    active: usize,
}

impl GenericRepr for TabsState {
    type Repr = Product<Field<usize>, Unit>;
    fn into_repr(self) -> Self::Repr {
        Product(Field::new("active", self.active), Unit)
    }
    fn from_repr(repr: Self::Repr) -> Self {
        Self {
            active: repr.0.value,
        }
    }
}

/// Simulates EditorState: cursor position + modified flag.
#[derive(Clone, Debug, PartialEq)]
struct EditorState {
    cursor_row: usize,
    cursor_col: usize,
    modified: bool,
}

impl GenericRepr for EditorState {
    type Repr = Product<Field<usize>, Product<Field<usize>, Product<Field<bool>, Unit>>>;
    fn into_repr(self) -> Self::Repr {
        Product(
            Field::new("cursor_row", self.cursor_row),
            Product(
                Field::new("cursor_col", self.cursor_col),
                Product(Field::new("modified", self.modified), Unit),
            ),
        )
    }
    fn from_repr(repr: Self::Repr) -> Self {
        Self {
            cursor_row: repr.0.value,
            cursor_col: repr.1.0.value,
            modified: repr.1.1.0.value,
        }
    }
}

/// Simulates TreeState: expanded/collapsed node + selection.
#[derive(Clone, Debug, PartialEq)]
enum TreeNodeState {
    Collapsed,
    Expanded,
    Selected,
}

impl GenericRepr for TreeNodeState {
    type Repr = Sum<Variant<Unit>, Sum<Variant<Unit>, Sum<Variant<Unit>, Void>>>;
    fn into_repr(self) -> Self::Repr {
        match self {
            Self::Collapsed => Sum::Left(Variant::new("Collapsed", Unit)),
            Self::Expanded => Sum::Right(Sum::Left(Variant::new("Expanded", Unit))),
            Self::Selected => Sum::Right(Sum::Right(Sum::Left(Variant::new("Selected", Unit)))),
        }
    }
    fn from_repr(repr: Self::Repr) -> Self {
        match repr {
            Sum::Left(_) => Self::Collapsed,
            Sum::Right(Sum::Left(_)) => Self::Expanded,
            Sum::Right(Sum::Right(Sum::Left(_))) => Self::Selected,
            Sum::Right(Sum::Right(Sum::Right(v))) => match v {},
        }
    }
}

/// Simulates ScrollbarState: position + viewport_size + content_size.
#[derive(Clone, Debug, PartialEq)]
struct ScrollbarState {
    position: usize,
    viewport: usize,
    content: usize,
}

impl GenericRepr for ScrollbarState {
    type Repr = Product<Field<usize>, Product<Field<usize>, Product<Field<usize>, Unit>>>;
    fn into_repr(self) -> Self::Repr {
        Product(
            Field::new("position", self.position),
            Product(
                Field::new("viewport", self.viewport),
                Product(Field::new("content", self.content), Unit),
            ),
        )
    }
    fn from_repr(repr: Self::Repr) -> Self {
        Self {
            position: repr.0.value,
            viewport: repr.1.0.value,
            content: repr.1.1.0.value,
        }
    }
}

// ── JSONL evidence record ────────────────────────────────────────

struct DiffPatchEvidence {
    widget_type: &'static str,
    scenario: &'static str,
    diff_change_count: usize,
    roundtrip_match: bool,
    diff_time_ns: u64,
    patch_time_ns: u64,
}

impl DiffPatchEvidence {
    fn to_jsonl(&self) -> String {
        format!(
            concat!(
                "{{\"event\":\"diffpatch_roundtrip\",",
                "\"widget_type\":\"{}\",",
                "\"scenario\":\"{}\",",
                "\"diff_change_count\":{},",
                "\"roundtrip_match\":{},",
                "\"diff_time_ns\":{},",
                "\"patch_time_ns\":{}}}"
            ),
            self.widget_type,
            self.scenario,
            self.diff_change_count,
            self.roundtrip_match,
            self.diff_time_ns,
            self.patch_time_ns,
        )
    }
}

/// Run a diff/patch round-trip test with evidence collection.
fn roundtrip_test<T>(
    widget_type: &'static str,
    scenario: &'static str,
    old: &T,
    new: &T,
) -> DiffPatchEvidence
where
    T: GenericRepr + Clone + PartialEq + std::fmt::Debug,
    T::Repr: Diff + Patch + Clone,
    <T::Repr as Diff>::Diff: DiffInfo,
{
    let start = Instant::now();
    let diff = generic_diff(old, new);
    let diff_time_ns = start.elapsed().as_nanos() as u64;

    let change_count = diff.change_count();

    let start = Instant::now();
    let patched = generic_patch(old, &diff);
    let patch_time_ns = start.elapsed().as_nanos() as u64;

    let roundtrip_match = patched == *new;

    DiffPatchEvidence {
        widget_type,
        scenario,
        diff_change_count: change_count,
        roundtrip_match,
        diff_time_ns,
        patch_time_ns,
    }
}

// ── E2E tests ────────────────────────────────────────────────────

#[test]
fn e2e_list_state_roundtrip() {
    let old = ListState {
        selected: 0,
        offset: 0,
        item_count: 100,
    };
    let new = ListState {
        selected: 5,
        offset: 3,
        item_count: 100,
    };
    let ev = roundtrip_test("ListState", "scroll_down", &old, &new);
    assert!(ev.roundtrip_match, "ListState roundtrip failed");
    assert_eq!(ev.diff_change_count, 2); // selected + offset changed
    eprintln!("{}", ev.to_jsonl());
}

#[test]
fn e2e_table_state_roundtrip() {
    let old = TableState {
        row: 0,
        col: 0,
        sort_col: 0,
        sort_asc: true,
    };
    let new = TableState {
        row: 10,
        col: 3,
        sort_col: 2,
        sort_asc: false,
    };
    let ev = roundtrip_test("TableState", "navigate_and_sort", &old, &new);
    assert!(ev.roundtrip_match, "TableState roundtrip failed");
    assert_eq!(ev.diff_change_count, 4); // all fields changed
    eprintln!("{}", ev.to_jsonl());
}

#[test]
fn e2e_tabs_state_roundtrip() {
    let old = TabsState { active: 0 };
    let new = TabsState { active: 3 };
    let ev = roundtrip_test("TabsState", "switch_tab", &old, &new);
    assert!(ev.roundtrip_match, "TabsState roundtrip failed");
    assert_eq!(ev.diff_change_count, 1);
    eprintln!("{}", ev.to_jsonl());
}

#[test]
fn e2e_editor_state_roundtrip() {
    let old = EditorState {
        cursor_row: 0,
        cursor_col: 0,
        modified: false,
    };
    let new = EditorState {
        cursor_row: 42,
        cursor_col: 15,
        modified: true,
    };
    let ev = roundtrip_test("EditorState", "edit_text", &old, &new);
    assert!(ev.roundtrip_match, "EditorState roundtrip failed");
    assert_eq!(ev.diff_change_count, 3);
    eprintln!("{}", ev.to_jsonl());
}

#[test]
fn e2e_tree_node_roundtrip() {
    let old = TreeNodeState::Collapsed;
    let new = TreeNodeState::Expanded;
    let ev = roundtrip_test("TreeNodeState", "expand_node", &old, &new);
    assert!(ev.roundtrip_match, "TreeNodeState roundtrip failed");
    assert_eq!(ev.diff_change_count, 1);
    eprintln!("{}", ev.to_jsonl());
}

#[test]
fn e2e_scrollbar_state_roundtrip() {
    let old = ScrollbarState {
        position: 0,
        viewport: 20,
        content: 100,
    };
    let new = ScrollbarState {
        position: 50,
        viewport: 20,
        content: 100,
    };
    let ev = roundtrip_test("ScrollbarState", "scroll_to_middle", &old, &new);
    assert!(ev.roundtrip_match, "ScrollbarState roundtrip failed");
    assert_eq!(ev.diff_change_count, 1); // only position changed
    eprintln!("{}", ev.to_jsonl());
}

// ── Identity and empty diff tests ────────────────────────────────

#[test]
fn e2e_identical_states_empty_diff() {
    let state = ListState {
        selected: 5,
        offset: 3,
        item_count: 100,
    };
    let diff = generic_diff(&state, &state);
    assert!(
        diff.is_empty(),
        "identical states should produce empty diff"
    );
    assert_eq!(diff.change_count(), 0);

    let patched = generic_patch(&state, &diff);
    assert_eq!(patched, state);
}

#[test]
fn e2e_identical_table_empty_diff() {
    let state = TableState {
        row: 10,
        col: 3,
        sort_col: 2,
        sort_asc: false,
    };
    let diff = generic_diff(&state, &state);
    assert!(diff.is_empty());
    assert_eq!(diff.change_count(), 0);
}

#[test]
fn e2e_identical_enum_empty_diff() {
    let state = TreeNodeState::Expanded;
    let diff = generic_diff(&state, &state);
    assert!(diff.is_empty());
}

// ── Maximal diff tests ───────────────────────────────────────────

#[test]
fn e2e_maximal_diff_list() {
    let old = ListState {
        selected: 0,
        offset: 0,
        item_count: 0,
    };
    let new = ListState {
        selected: 999,
        offset: 500,
        item_count: 1000,
    };
    let diff = generic_diff(&old, &new);
    assert_eq!(diff.change_count(), 3, "all 3 fields should differ");
    let patched = generic_patch(&old, &diff);
    assert_eq!(patched, new);
}

#[test]
fn e2e_maximal_diff_editor() {
    let old = EditorState {
        cursor_row: 0,
        cursor_col: 0,
        modified: false,
    };
    let new = EditorState {
        cursor_row: usize::MAX,
        cursor_col: usize::MAX,
        modified: true,
    };
    let diff = generic_diff(&old, &new);
    assert_eq!(diff.change_count(), 3);
    let patched = generic_patch(&old, &diff);
    assert_eq!(patched, new);
}

// ── Variant change E2E ───────────────────────────────────────────

#[test]
fn e2e_all_tree_transitions() {
    let variants = [
        TreeNodeState::Collapsed,
        TreeNodeState::Expanded,
        TreeNodeState::Selected,
    ];
    for old in &variants {
        for new in &variants {
            let diff = generic_diff(old, new);
            let patched = generic_patch(old, &diff);
            assert_eq!(&patched, new, "roundtrip failed for {old:?} -> {new:?}");
        }
    }
}

// ── Performance: diff and patch under 1ms ────────────────────────

#[test]
fn e2e_performance_under_1ms() {
    let old = TableState {
        row: 0,
        col: 0,
        sort_col: 0,
        sort_asc: true,
    };
    let new = TableState {
        row: 100,
        col: 50,
        sort_col: 5,
        sort_asc: false,
    };

    // Warm up
    for _ in 0..100 {
        let _ = generic_diff(&old, &new);
    }

    let start = Instant::now();
    for _ in 0..1000 {
        let diff = generic_diff(&old, &new);
        let _ = generic_patch(&old, &diff);
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / 1000;

    assert!(
        per_op.as_micros() < 1000,
        "diff+patch should be < 1ms, got {:?}",
        per_op
    );
}

// ── JSONL schema compliance ──────────────────────────────────────

#[test]
fn e2e_jsonl_schema_compliance() {
    let old = ListState {
        selected: 0,
        offset: 0,
        item_count: 100,
    };
    let new = ListState {
        selected: 5,
        offset: 3,
        item_count: 100,
    };
    let ev = roundtrip_test("ListState", "schema_test", &old, &new);
    let jsonl = ev.to_jsonl();

    // Verify required fields are present
    assert!(jsonl.contains("\"event\":\"diffpatch_roundtrip\""));
    assert!(jsonl.contains("\"widget_type\":\"ListState\""));
    assert!(jsonl.contains("\"scenario\":\"schema_test\""));
    assert!(jsonl.contains("\"diff_change_count\":"));
    assert!(jsonl.contains("\"roundtrip_match\":true"));
    assert!(jsonl.contains("\"diff_time_ns\":"));
    assert!(jsonl.contains("\"patch_time_ns\":"));

    // Verify it's valid JSON-ish (starts/ends with braces)
    assert!(jsonl.starts_with('{'));
    assert!(jsonl.ends_with('}'));
}

// ── Comprehensive all-state roundtrip ────────────────────────────

#[test]
fn e2e_all_widget_states_roundtrip() {
    let evidence = vec![
        roundtrip_test(
            "ListState",
            "initial_to_scrolled",
            &ListState {
                selected: 0,
                offset: 0,
                item_count: 100,
            },
            &ListState {
                selected: 20,
                offset: 15,
                item_count: 100,
            },
        ),
        roundtrip_test(
            "TableState",
            "navigate_sort",
            &TableState {
                row: 0,
                col: 0,
                sort_col: 0,
                sort_asc: true,
            },
            &TableState {
                row: 50,
                col: 5,
                sort_col: 3,
                sort_asc: false,
            },
        ),
        roundtrip_test(
            "TabsState",
            "switch",
            &TabsState { active: 0 },
            &TabsState { active: 4 },
        ),
        roundtrip_test(
            "EditorState",
            "typing",
            &EditorState {
                cursor_row: 0,
                cursor_col: 0,
                modified: false,
            },
            &EditorState {
                cursor_row: 10,
                cursor_col: 25,
                modified: true,
            },
        ),
        roundtrip_test(
            "TreeNodeState",
            "collapse_to_expand",
            &TreeNodeState::Collapsed,
            &TreeNodeState::Expanded,
        ),
        roundtrip_test(
            "ScrollbarState",
            "scroll",
            &ScrollbarState {
                position: 0,
                viewport: 20,
                content: 200,
            },
            &ScrollbarState {
                position: 180,
                viewport: 20,
                content: 200,
            },
        ),
    ];

    // Verify ALL round-trips succeeded
    for ev in &evidence {
        assert!(
            ev.roundtrip_match,
            "failed for {}/{}",
            ev.widget_type, ev.scenario
        );
    }

    // Emit JSONL
    eprintln!("--- JSONL Evidence ---");
    for ev in &evidence {
        eprintln!("{}", ev.to_jsonl());
    }
}
