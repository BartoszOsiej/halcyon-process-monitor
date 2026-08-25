#![forbid(unsafe_code)]

//! Cache-oblivious van Emde Boas tree layout for widget traversal (bd-xwkon).
//!
//! Flattens a logical tree into a contiguous `Vec` using the recursive van Emde
//! Boas (vEB) memory layout. This ensures `O(log_B(n))` cache misses per
//! root-to-leaf traversal for any cache line size `B`, without knowing `B` at
//! compile time.
//!
//! # Overview
//!
//! A standard BFS or DFS layout puts sibling nodes far apart in memory for
//! deep trees. The vEB layout recursively stores the top half of the tree
//! followed by each bottom subtree, keeping ancestors and descendants
//! close together in memory.
//!
//! # Usage
//!
//! ```
//! use ftui_layout::veb_tree::{VebTree, TreeNode};
//!
//! // Build a small tree: root with two children
//! let nodes = vec![
//!     TreeNode::new(0, "root", vec![1, 2]),
//!     TreeNode::new(1, "left", vec![]),
//!     TreeNode::new(2, "right", vec![]),
//! ];
//! let tree = VebTree::build(nodes);
//! assert_eq!(tree.len(), 3);
//! assert_eq!(tree.get(0).unwrap().data, "root");
//! ```

use std::collections::HashMap;

/// A node in the logical tree before vEB layout.
#[derive(Debug, Clone)]
pub struct TreeNode<T> {
    /// Unique node identifier.
    pub id: u32,
    /// User data (e.g., widget state, layout constraints).
    pub data: T,
    /// IDs of child nodes (order preserved).
    pub children: Vec<u32>,
}

impl<T> TreeNode<T> {
    /// Create a new tree node.
    pub fn new(id: u32, data: T, children: Vec<u32>) -> Self {
        Self { id, data, children }
    }
}

/// A node stored in the vEB-laid-out flat array.
#[derive(Debug, Clone)]
pub struct VebEntry<T> {
    /// Original node ID.
    pub id: u32,
    /// User data.
    pub data: T,
    /// Indices of children in the flat array (not node IDs).
    pub child_indices: Vec<u32>,
    /// Index of parent in the flat array (`u32::MAX` for root).
    pub parent_index: u32,
    /// Depth in the original tree (root = 0).
    pub depth: u16,
}

/// A tree stored in van Emde Boas memory layout for cache-oblivious traversal.
#[derive(Debug, Clone)]
pub struct VebTree<T> {
    /// Flat array of nodes in vEB order.
    nodes: Vec<VebEntry<T>>,
    /// Map from node ID → position in `nodes`.
    index: HashMap<u32, u32>,
}

impl<T: Clone> VebTree<T> {
    /// Build a `VebTree` from a list of logical tree nodes.
    ///
    /// The first node in `input` whose `id` does not appear as any other
    /// node's child is treated as root. If `input` is empty, returns an
    /// empty tree.
    pub fn build(input: Vec<TreeNode<T>>) -> Self {
        if input.is_empty() {
            return Self {
                nodes: Vec::new(),
                index: HashMap::new(),
            };
        }

        // Index nodes by ID.
        let node_map: HashMap<u32, &TreeNode<T>> = input.iter().map(|n| (n.id, n)).collect();

        // Find root: a node whose ID is not a child of any other node.
        let all_children: std::collections::HashSet<u32> = input
            .iter()
            .flat_map(|n| n.children.iter().copied())
            .collect();
        let root_id = input
            .iter()
            .find(|n| !all_children.contains(&n.id))
            .map(|n| n.id)
            .unwrap_or(input[0].id);

        // Compute depths via BFS.
        let mut depths: HashMap<u32, u16> = HashMap::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((root_id, 0u16));
        while let Some((nid, d)) = queue.pop_front() {
            depths.insert(nid, d);
            if let Some(node) = node_map.get(&nid) {
                for &cid in &node.children {
                    queue.push_back((cid, d + 1));
                }
            }
        }

        // Collect DFS order (used as input to vEB layout).
        let mut dfs_order: Vec<u32> = Vec::with_capacity(input.len());
        let mut stack = vec![root_id];
        while let Some(nid) = stack.pop() {
            dfs_order.push(nid);
            if let Some(node) = node_map.get(&nid) {
                // Push children in reverse so leftmost is visited first.
                for &cid in node.children.iter().rev() {
                    stack.push(cid);
                }
            }
        }

        // Apply vEB recursive layout.
        let veb_order = veb_layout_order(&dfs_order, &node_map);

        // Build the flat array.
        let mut id_to_pos: HashMap<u32, u32> = HashMap::with_capacity(veb_order.len());
        for (pos, &nid) in veb_order.iter().enumerate() {
            id_to_pos.insert(nid, pos as u32);
        }

        // Build parent map.
        let mut parent_map: HashMap<u32, u32> = HashMap::new();
        for node in &input {
            for &cid in &node.children {
                parent_map.insert(cid, node.id);
            }
        }

        let nodes: Vec<VebEntry<T>> = veb_order
            .iter()
            .map(|&nid| {
                let node = node_map[&nid];
                let child_indices: Vec<u32> = node
                    .children
                    .iter()
                    .filter_map(|cid| id_to_pos.get(cid).copied())
                    .collect();
                let parent_index = parent_map
                    .get(&nid)
                    .and_then(|pid| id_to_pos.get(pid).copied())
                    .unwrap_or(u32::MAX);
                VebEntry {
                    id: nid,
                    data: node.data.clone(),
                    child_indices,
                    parent_index,
                    depth: depths.get(&nid).copied().unwrap_or(0),
                }
            })
            .collect();

        Self {
            nodes,
            index: id_to_pos,
        }
    }

    /// Number of nodes.
    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the tree is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Look up a node by its original ID. O(1) via hash map.
    #[inline]
    pub fn get(&self, id: u32) -> Option<&VebEntry<T>> {
        self.index.get(&id).map(|&pos| &self.nodes[pos as usize])
    }

    /// Look up a node by its position in the flat array.
    #[inline]
    pub fn get_by_index(&self, idx: u32) -> Option<&VebEntry<T>> {
        self.nodes.get(idx as usize)
    }

    /// Iterate nodes in vEB order (cache-friendly traversal).
    pub fn iter(&self) -> impl Iterator<Item = &VebEntry<T>> {
        self.nodes.iter()
    }

    /// Iterate nodes in DFS pre-order (logical traversal order).
    pub fn iter_dfs(&self) -> Vec<&VebEntry<T>> {
        if self.nodes.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(self.nodes.len());
        let mut stack = vec![0u32]; // Root is first in vEB layout
        while let Some(idx) = stack.pop() {
            if let Some(entry) = self.nodes.get(idx as usize) {
                result.push(entry);
                for &ci in entry.child_indices.iter().rev() {
                    stack.push(ci);
                }
            }
        }
        result
    }

    /// Get the root entry (always at position 0 if tree is non-empty).
    pub fn root(&self) -> Option<&VebEntry<T>> {
        self.nodes.first()
    }

    /// Return the raw flat array slice.
    pub fn as_slice(&self) -> &[VebEntry<T>] {
        &self.nodes
    }
}

/// Compute the van Emde Boas layout order for a set of node IDs.
///
/// The algorithm:
/// 1. Assign each node a DFS rank.
/// 2. Recursively split the tree at the median depth.
/// 3. Place the top half, then each bottom subtree.
fn veb_layout_order<T>(dfs_order: &[u32], node_map: &HashMap<u32, &TreeNode<T>>) -> Vec<u32> {
    if dfs_order.len() <= 1 {
        return dfs_order.to_vec();
    }

    // Compute depth for each node in this subtree.
    let root = dfs_order[0];
    let mut depths: HashMap<u32, u16> = HashMap::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((root, 0u16));
    let subtree_set: std::collections::HashSet<u32> = dfs_order.iter().copied().collect();
    while let Some((nid, d)) = queue.pop_front() {
        depths.insert(nid, d);
        if let Some(node) = node_map.get(&nid) {
            for &cid in &node.children {
                if subtree_set.contains(&cid) {
                    queue.push_back((cid, d + 1));
                }
            }
        }
    }

    let max_depth = depths.values().copied().max().unwrap_or(0);
    if max_depth <= 1 {
        // Tree is flat enough — just return DFS order.
        return dfs_order.to_vec();
    }

    let mid_depth = max_depth / 2;

    // Split into top (depth <= mid_depth) and bottom subtrees.
    let mut top: Vec<u32> = Vec::new();
    let mut bottom_roots: Vec<u32> = Vec::new();
    let mut bottom_subtrees: HashMap<u32, Vec<u32>> = HashMap::new();

    for &nid in dfs_order {
        let d = depths.get(&nid).copied().unwrap_or(0);
        if d <= mid_depth {
            top.push(nid);
            // Check if any child is in the bottom half.
            if let Some(node) = node_map.get(&nid) {
                for &cid in &node.children {
                    if subtree_set.contains(&cid) {
                        let cd = depths.get(&cid).copied().unwrap_or(0);
                        if cd > mid_depth {
                            bottom_roots.push(cid);
                        }
                    }
                }
            }
        }
    }

    // Build each bottom subtree's DFS order.
    for &br in &bottom_roots {
        let mut subtree = Vec::new();
        let mut stack = vec![br];
        while let Some(nid) = stack.pop() {
            if subtree_set.contains(&nid) {
                subtree.push(nid);
                if let Some(node) = node_map.get(&nid) {
                    for &cid in node.children.iter().rev() {
                        if subtree_set.contains(&cid) {
                            stack.push(cid);
                        }
                    }
                }
            }
        }
        bottom_subtrees.insert(br, subtree);
    }

    // Recursively layout top and each bottom subtree.
    let mut result = veb_layout_order(&top, node_map);
    for &br in &bottom_roots {
        if let Some(subtree) = bottom_subtrees.get(&br) {
            result.extend(veb_layout_order(subtree, node_map));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_binary_tree(depth: u16) -> Vec<TreeNode<String>> {
        let mut nodes = Vec::new();
        let mut next_id = 1u32;
        fn build(
            id: u32,
            depth: u16,
            remaining: u16,
            next_id: &mut u32,
            nodes: &mut Vec<TreeNode<String>>,
        ) {
            let label = format!("node_{id}_d{depth}");
            if remaining == 0 {
                nodes.push(TreeNode::new(id, label, vec![]));
                return;
            }
            let left = *next_id;
            *next_id += 1;
            let right = *next_id;
            *next_id += 1;
            nodes.push(TreeNode::new(id, label, vec![left, right]));
            build(left, depth + 1, remaining - 1, next_id, nodes);
            build(right, depth + 1, remaining - 1, next_id, nodes);
        }
        build(0, 0, depth, &mut next_id, &mut nodes);
        nodes
    }

    #[test]
    fn empty_tree() {
        let tree: VebTree<&str> = VebTree::build(vec![]);
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);
        assert!(tree.root().is_none());
    }

    #[test]
    fn single_node() {
        let tree = VebTree::build(vec![TreeNode::new(42, "solo", vec![])]);
        assert_eq!(tree.len(), 1);
        let root = tree.root().unwrap();
        assert_eq!(root.id, 42);
        assert_eq!(root.data, "solo");
        assert!(root.child_indices.is_empty());
        assert_eq!(root.parent_index, u32::MAX);
    }

    #[test]
    fn three_node_tree() {
        let nodes = vec![
            TreeNode::new(0, "root", vec![1, 2]),
            TreeNode::new(1, "left", vec![]),
            TreeNode::new(2, "right", vec![]),
        ];
        let tree = VebTree::build(nodes);
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.get(0).unwrap().data, "root");
        assert_eq!(tree.get(1).unwrap().data, "left");
        assert_eq!(tree.get(2).unwrap().data, "right");
    }

    #[test]
    fn lookup_by_id() {
        let nodes = vec![
            TreeNode::new(10, "a", vec![20, 30]),
            TreeNode::new(20, "b", vec![]),
            TreeNode::new(30, "c", vec![]),
        ];
        let tree = VebTree::build(nodes);
        assert_eq!(tree.get(10).unwrap().data, "a");
        assert_eq!(tree.get(20).unwrap().data, "b");
        assert_eq!(tree.get(30).unwrap().data, "c");
        assert!(tree.get(99).is_none());
    }

    #[test]
    fn parent_indices_correct() {
        let nodes = vec![
            TreeNode::new(0, "r", vec![1, 2]),
            TreeNode::new(1, "l", vec![3]),
            TreeNode::new(2, "r2", vec![]),
            TreeNode::new(3, "ll", vec![]),
        ];
        let tree = VebTree::build(nodes);
        let root = tree.get(0).unwrap();
        assert_eq!(root.parent_index, u32::MAX);

        let left = tree.get(1).unwrap();
        let root_pos = tree.index[&0];
        assert_eq!(left.parent_index, root_pos);

        let ll = tree.get(3).unwrap();
        let left_pos = tree.index[&1];
        assert_eq!(ll.parent_index, left_pos);
    }

    #[test]
    fn child_indices_correct() {
        let nodes = vec![
            TreeNode::new(0, "r", vec![1, 2]),
            TreeNode::new(1, "l", vec![]),
            TreeNode::new(2, "r2", vec![]),
        ];
        let tree = VebTree::build(nodes);
        let root = tree.get(0).unwrap();
        assert_eq!(root.child_indices.len(), 2);

        // Children should be reachable via their indices.
        for &ci in &root.child_indices {
            let child = tree.get_by_index(ci).unwrap();
            assert!(child.id == 1 || child.id == 2);
        }
    }

    #[test]
    fn dfs_iteration_preserves_all_nodes() {
        let nodes = make_binary_tree(3);
        let count = nodes.len();
        let tree = VebTree::build(nodes);
        let dfs = tree.iter_dfs();
        assert_eq!(dfs.len(), count);

        // All IDs present.
        let mut ids: Vec<u32> = dfs.iter().map(|e| e.id).collect();
        ids.sort();
        let mut expected: Vec<u32> = (0..count as u32).collect();
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn dfs_root_first() {
        let nodes = make_binary_tree(3);
        let tree = VebTree::build(nodes);
        let dfs = tree.iter_dfs();
        assert_eq!(dfs[0].id, 0); // Root first
    }

    #[test]
    fn veb_order_contains_all_nodes() {
        let nodes = make_binary_tree(4);
        let count = nodes.len();
        let tree = VebTree::build(nodes);
        assert_eq!(tree.len(), count);

        // Iterate in vEB order.
        let veb_ids: Vec<u32> = tree.iter().map(|e| e.id).collect();
        assert_eq!(veb_ids.len(), count);

        // All IDs present.
        let mut sorted = veb_ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), count);
    }

    #[test]
    fn depth_values_correct() {
        let nodes = vec![
            TreeNode::new(0, "d0", vec![1, 2]),
            TreeNode::new(1, "d1a", vec![3]),
            TreeNode::new(2, "d1b", vec![]),
            TreeNode::new(3, "d2", vec![]),
        ];
        let tree = VebTree::build(nodes);
        assert_eq!(tree.get(0).unwrap().depth, 0);
        assert_eq!(tree.get(1).unwrap().depth, 1);
        assert_eq!(tree.get(2).unwrap().depth, 1);
        assert_eq!(tree.get(3).unwrap().depth, 2);
    }

    #[test]
    fn large_tree_1000_nodes() {
        // Linear chain of 1000 nodes.
        let nodes: Vec<TreeNode<u32>> = (0..1000)
            .map(|i| {
                let children = if i < 999 { vec![i + 1] } else { vec![] };
                TreeNode::new(i, i, children)
            })
            .collect();
        let tree = VebTree::build(nodes);
        assert_eq!(tree.len(), 1000);
        assert_eq!(tree.get(0).unwrap().depth, 0);
        assert_eq!(tree.get(999).unwrap().depth, 999);
    }

    #[test]
    fn layout_results_identical() {
        // Property: vEB layout and DFS should visit exactly the same nodes.
        let nodes = make_binary_tree(4);
        let tree = VebTree::build(nodes);

        let veb_ids: std::collections::HashSet<u32> = tree.iter().map(|e| e.id).collect();
        let dfs_ids: std::collections::HashSet<u32> =
            tree.iter_dfs().iter().map(|e| e.id).collect();
        assert_eq!(veb_ids, dfs_ids);
    }

    #[test]
    fn wide_tree() {
        // Root with 100 leaf children.
        let mut nodes = vec![TreeNode::new(0, 0u32, (1..=100).collect())];
        for i in 1..=100 {
            nodes.push(TreeNode::new(i, i, vec![]));
        }
        let tree = VebTree::build(nodes);
        assert_eq!(tree.len(), 101);
        assert_eq!(tree.get(0).unwrap().child_indices.len(), 100);
    }

    #[test]
    fn rebuild_produces_same_result() {
        let nodes = make_binary_tree(3);
        let tree1 = VebTree::build(nodes.clone());
        let tree2 = VebTree::build(nodes);

        let ids1: Vec<u32> = tree1.iter().map(|e| e.id).collect();
        let ids2: Vec<u32> = tree2.iter().map(|e| e.id).collect();
        assert_eq!(ids1, ids2);
    }
}
