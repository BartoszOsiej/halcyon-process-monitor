//! Property-based invariant tests for the van Emde Boas tree layout (bd-xwkon).
//!
//! Verifies that the vEB layout preserves all tree properties regardless
//! of the input tree's shape or size:
//!
//! 1. Node count conservation: vEB layout contains exactly the same nodes.
//! 2. ID uniqueness: no duplicate node IDs in the flat array.
//! 3. Root is first: root node is always at position 0.
//! 4. Parent-child consistency: child_indices and parent_index agree.
//! 5. Depth correctness: depth values match tree structure.
//! 6. DFS traversal completeness: iter_dfs visits all nodes.
//! 7. Lookup correctness: get(id) returns the correct node for all IDs.
//! 8. Determinism: building from the same input twice yields identical layout.

use ftui_layout::veb_tree::{TreeNode, VebTree};
use proptest::prelude::*;
use std::collections::{HashMap, HashSet};

// ── Strategies ────────────────────────────────────────────────────────────

/// Generate a random tree with `n` nodes.
fn tree_strategy(max_nodes: usize) -> impl Strategy<Value = Vec<TreeNode<u32>>> {
    (2..=max_nodes).prop_flat_map(|n| {
        // For each non-root node, choose a parent from 0..i.
        let parent_choices: Vec<_> = (1..n).map(|i| 0..i).collect();
        parent_choices.prop_map(move |parents| {
            let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
            for (i, &parent) in parents.iter().enumerate() {
                let child_id = (i + 1) as u32;
                children.entry(parent as u32).or_default().push(child_id);
            }
            (0..n as u32)
                .map(|id| {
                    let kids = children.get(&id).cloned().unwrap_or_default();
                    TreeNode::new(id, id, kids)
                })
                .collect()
        })
    })
}

/// Generate a balanced binary tree of given depth.
fn binary_tree_strategy() -> impl Strategy<Value = Vec<TreeNode<u32>>> {
    (1..8u16).prop_map(|depth| {
        let mut nodes = Vec::new();
        let mut next_id = 1u32;
        fn build(id: u32, remaining: u16, next_id: &mut u32, nodes: &mut Vec<TreeNode<u32>>) {
            if remaining == 0 {
                nodes.push(TreeNode::new(id, id, vec![]));
                return;
            }
            let left = *next_id;
            *next_id += 1;
            let right = *next_id;
            *next_id += 1;
            nodes.push(TreeNode::new(id, id, vec![left, right]));
            build(left, remaining - 1, next_id, nodes);
            build(right, remaining - 1, next_id, nodes);
        }
        build(0, depth, &mut next_id, &mut nodes);
        nodes
    })
}

// ── Property Tests ────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn veb_node_count_conservation(nodes in tree_strategy(100)) {
        let n = nodes.len();
        let tree = VebTree::build(nodes);
        prop_assert_eq!(tree.len(), n);
    }

    #[test]
    fn veb_id_uniqueness(nodes in tree_strategy(100)) {
        let tree = VebTree::build(nodes);
        let ids: Vec<u32> = tree.iter().map(|e| e.id).collect();
        let unique: HashSet<u32> = ids.iter().copied().collect();
        prop_assert_eq!(ids.len(), unique.len(), "duplicate IDs in vEB layout");
    }

    #[test]
    fn veb_root_is_first(nodes in tree_strategy(100)) {
        let tree = VebTree::build(nodes);
        let root = tree.root().unwrap();
        prop_assert_eq!(root.id, 0, "root should be node 0");
        prop_assert_eq!(root.parent_index, u32::MAX, "root should have no parent");
    }

    #[test]
    fn veb_parent_child_consistency(nodes in tree_strategy(50)) {
        let tree = VebTree::build(nodes);
        for (pos, entry) in tree.as_slice().iter().enumerate() {
            // Each child's parent should point back to this node.
            for &ci in &entry.child_indices {
                let child = tree.get_by_index(ci).unwrap();
                prop_assert_eq!(
                    child.parent_index, pos as u32,
                    "child {} parent_index mismatch", child.id
                );
            }
            // Parent should list this node as a child.
            if entry.parent_index != u32::MAX {
                let parent = tree.get_by_index(entry.parent_index).unwrap();
                prop_assert!(
                    parent.child_indices.contains(&(pos as u32)),
                    "node {} not in parent's child list", entry.id
                );
            }
        }
    }

    #[test]
    fn veb_depth_correctness(nodes in tree_strategy(50)) {
        let tree = VebTree::build(nodes);
        let root = tree.root().unwrap();
        prop_assert_eq!(root.depth, 0);
        // Each child's depth should be parent's depth + 1.
        for entry in tree.iter() {
            for &ci in &entry.child_indices {
                let child = tree.get_by_index(ci).unwrap();
                prop_assert_eq!(
                    child.depth, entry.depth + 1,
                    "depth mismatch: child {} depth={} parent {} depth={}",
                    child.id, child.depth, entry.id, entry.depth
                );
            }
        }
    }

    #[test]
    fn veb_dfs_completeness(nodes in tree_strategy(100)) {
        let n = nodes.len();
        let tree = VebTree::build(nodes);
        let dfs = tree.iter_dfs();
        prop_assert_eq!(dfs.len(), n, "DFS should visit all nodes");
        let dfs_ids: HashSet<u32> = dfs.iter().map(|e| e.id).collect();
        let all_ids: HashSet<u32> = tree.iter().map(|e| e.id).collect();
        prop_assert_eq!(dfs_ids, all_ids);
    }

    #[test]
    fn veb_lookup_correctness(nodes in tree_strategy(100)) {
        let input_data: HashMap<u32, u32> = nodes.iter().map(|n| (n.id, n.data)).collect();
        let tree = VebTree::build(nodes);
        for (&id, &expected) in &input_data {
            let entry = tree.get(id);
            prop_assert!(entry.is_some(), "get({}) returned None", id);
            prop_assert_eq!(entry.unwrap().data, expected);
        }
    }

    #[test]
    fn veb_deterministic(nodes in tree_strategy(50)) {
        let tree1 = VebTree::build(nodes.clone());
        let tree2 = VebTree::build(nodes);
        let ids1: Vec<u32> = tree1.iter().map(|e| e.id).collect();
        let ids2: Vec<u32> = tree2.iter().map(|e| e.id).collect();
        prop_assert_eq!(ids1, ids2);
    }

    #[test]
    fn veb_binary_tree_invariants(nodes in binary_tree_strategy()) {
        let n = nodes.len();
        let tree = VebTree::build(nodes);
        prop_assert_eq!(tree.len(), n);

        // Binary tree: each internal node has exactly 2 children.
        for entry in tree.iter() {
            let cc = entry.child_indices.len();
            prop_assert!(cc == 0 || cc == 2, "binary node {} has {} children", entry.id, cc);
        }
    }
}
