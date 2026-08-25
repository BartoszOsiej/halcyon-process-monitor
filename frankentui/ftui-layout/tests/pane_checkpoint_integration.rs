use std::collections::BTreeMap;

use ftui_layout::{
    PANE_TREE_SCHEMA_VERSION, PaneId, PaneInteractionTimeline, PaneLeaf, PaneNodeKind,
    PaneNodeRecord, PaneOperation, PaneSplit, PaneSplitRatio, PaneTree, PaneTreeSnapshot,
    SplitAxis, WorkspaceMetadata, workspace::WorkspaceSnapshot,
};

fn split_tree_snapshot() -> PaneTreeSnapshot {
    let root_id = PaneId::new(1).expect("root id");
    let left_id = PaneId::new(2).expect("left id");
    let right_id = PaneId::new(3).expect("right id");
    PaneTreeSnapshot {
        schema_version: PANE_TREE_SCHEMA_VERSION,
        root: root_id,
        next_id: PaneId::new(4).expect("next id"),
        nodes: vec![
            PaneNodeRecord::split(
                root_id,
                None,
                PaneSplit {
                    axis: SplitAxis::Horizontal,
                    ratio: PaneSplitRatio::new(1, 1).expect("valid ratio"),
                    first: left_id,
                    second: right_id,
                },
            ),
            PaneNodeRecord::leaf(left_id, Some(root_id), PaneLeaf::new("left")),
            PaneNodeRecord::leaf(right_id, Some(root_id), PaneLeaf::new("right")),
        ],
        extensions: BTreeMap::new(),
    }
}

fn root_split_id(tree: &PaneTree) -> PaneId {
    tree.nodes()
        .find_map(|node| matches!(node.kind, PaneNodeKind::Split(_)).then_some(node.id))
        .expect("split tree should contain one split")
}

#[test]
fn checkpointed_timeline_matches_baseline_timeline_across_branching_history() {
    let baseline_snapshot = split_tree_snapshot();
    let baseline_tree = PaneTree::from_snapshot(baseline_snapshot.clone()).expect("valid tree");

    let mut checkpointed_tree =
        PaneTree::from_snapshot(baseline_snapshot.clone()).expect("valid checkpointed tree");
    let mut baseline_replay_tree =
        PaneTree::from_snapshot(baseline_snapshot).expect("valid baseline replay tree");

    let mut checkpointed = PaneInteractionTimeline::with_baseline(&baseline_tree);
    checkpointed.checkpoint_interval = 4;
    let mut baseline = PaneInteractionTimeline::with_baseline(&baseline_tree);
    baseline.checkpoint_interval = usize::MAX;

    let split_id = root_split_id(&checkpointed_tree);
    let operation_plan = [
        PaneSplitRatio::new(3, 2).expect("valid ratio"),
        PaneSplitRatio::new(2, 3).expect("valid ratio"),
        PaneSplitRatio::new(5, 4).expect("valid ratio"),
        PaneSplitRatio::new(4, 5).expect("valid ratio"),
    ];

    for idx in 0..18u64 {
        let operation = PaneOperation::SetSplitRatio {
            split: split_id,
            ratio: operation_plan[idx as usize % operation_plan.len()],
        };
        checkpointed
            .apply_and_record(&mut checkpointed_tree, idx, 10_000 + idx, operation.clone())
            .expect("checkpointed apply should succeed");
        baseline
            .apply_and_record(&mut baseline_replay_tree, idx, 20_000 + idx, operation)
            .expect("baseline apply should succeed");
    }

    assert!(!checkpointed.checkpoints.is_empty());
    assert!(baseline.checkpoints.is_empty());
    assert_eq!(
        checkpointed_tree.state_hash(),
        baseline_replay_tree.state_hash()
    );
    assert_eq!(
        checkpointed
            .replay()
            .expect("checkpointed replay")
            .state_hash(),
        baseline.replay().expect("baseline replay").state_hash()
    );

    for _ in 0..5 {
        assert!(
            checkpointed
                .undo(&mut checkpointed_tree)
                .expect("checkpointed undo")
        );
        assert!(
            baseline
                .undo(&mut baseline_replay_tree)
                .expect("baseline undo")
        );
        assert_eq!(
            checkpointed_tree.state_hash(),
            baseline_replay_tree.state_hash()
        );
    }

    for _ in 0..3 {
        assert!(
            checkpointed
                .redo(&mut checkpointed_tree)
                .expect("checkpointed redo")
        );
        assert!(
            baseline
                .redo(&mut baseline_replay_tree)
                .expect("baseline redo")
        );
        assert_eq!(
            checkpointed_tree.state_hash(),
            baseline_replay_tree.state_hash()
        );
    }

    let branch_operation = PaneOperation::SetSplitRatio {
        split: split_id,
        ratio: PaneSplitRatio::new(7, 5).expect("valid ratio"),
    };
    checkpointed
        .apply_and_record(
            &mut checkpointed_tree,
            100,
            30_100,
            branch_operation.clone(),
        )
        .expect("checkpointed branch apply should succeed");
    baseline
        .apply_and_record(&mut baseline_replay_tree, 100, 40_100, branch_operation)
        .expect("baseline branch apply should succeed");

    assert_eq!(checkpointed.cursor, baseline.cursor);
    assert_eq!(checkpointed.applied_len(), baseline.applied_len());
    assert_eq!(
        checkpointed_tree.state_hash(),
        baseline_replay_tree.state_hash()
    );
    assert_eq!(
        checkpointed
            .replay()
            .expect("checkpointed replay")
            .to_snapshot(),
        baseline.replay().expect("baseline replay").to_snapshot()
    );

    let mut workspace = WorkspaceSnapshot::new(
        checkpointed_tree.to_snapshot(),
        WorkspaceMetadata::new("checkpointed"),
    );
    workspace.interaction_timeline = checkpointed;
    assert!(workspace.validate().is_ok());
}
