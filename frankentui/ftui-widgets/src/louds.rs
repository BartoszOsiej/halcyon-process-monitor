//! LOUDS (Level-Order Unary Degree Sequence) tree encoding.
//!
//! Compacts a tree structure from O(n × ptr_size) to 2n+1 bits while
//! supporting O(1) parent, first_child, next_sibling, and is_leaf
//! navigation via rank/select on a bitvector.
//!
//! # Encoding
//!
//! Traverse the tree in level order (BFS). For each node with `d` children,
//! emit `d` one-bits followed by a zero-bit. Prepend a sentinel `10` for
//! the super-root. Total bits: `2n + 1` for `n` nodes.
//!
//! # Navigation
//!
//! All navigation is O(1) via rank/select on the bitvector:
//! - `parent(v)`: `select1(rank0(v) - 1)`
//! - `first_child(v)`: `select0(rank1(v)) + 1`
//! - `next_sibling(v)`: `v + 1` (if bit at `v + 1` is `1`)
//! - `is_leaf(v)`: bit at `first_child(v)` is `0` or past end
//!
//! # Example
//!
//! ```
//! use ftui_widgets::louds::LoudsTree;
//!
//! // Build a tree:
//! //       root (0)
//! //      /    \
//! //    a (1)   b (2)
//! //    |
//! //    c (3)
//! let louds = LoudsTree::from_degrees(&[2, 1, 0, 0]);
//!
//! assert_eq!(louds.node_count(), 4);
//! assert_eq!(louds.first_child(0), Some(1));
//! assert_eq!(louds.next_sibling(1), Some(2));
//! assert_eq!(louds.first_child(1), Some(3));
//! assert!(louds.is_leaf(2));
//! assert!(louds.is_leaf(3));
//! assert_eq!(louds.parent(1), Some(0));
//! assert_eq!(louds.parent(3), Some(1));
//! ```

/// Number of `u64` words per rank superblock (512 bits).
const SUPERBLOCK_WORDS: usize = 8;

/// LOUDS-encoded tree with O(1) navigation.
///
/// Stores the tree structure in 2n+1 bits plus rank superblocks for fast
/// rank/select queries.
#[derive(Debug, Clone)]
pub struct LoudsTree {
    /// The LOUDS bitvector (including super-root sentinel).
    bits: Vec<u64>,
    /// Cumulative popcount at superblock boundaries.
    rank_superblocks: Vec<u64>,
    /// Total number of bits in the bitvector.
    bit_len: usize,
    /// Number of tree nodes (excluding the super-root).
    n: usize,
}

impl LoudsTree {
    /// Build a LOUDS tree from BFS-order degree sequence.
    ///
    /// `degrees[i]` is the number of children of the `i`-th node in
    /// level-order. The root is `degrees[0]`.
    ///
    /// # Panics
    ///
    /// Panics if the degree sequence is empty or inconsistent (doesn't
    /// describe a valid tree).
    pub fn from_degrees(degrees: &[usize]) -> Self {
        assert!(!degrees.is_empty(), "degree sequence must not be empty");

        let n = degrees.len();
        // Total bits: super-root (1,0) + for each node d_i ones + one zero = 2 + sum(d_i) + n
        // Since sum(d_i) = n - 1 for a tree: total = 2 + (n - 1) + n = 2n + 1
        let total_children: usize = degrees.iter().sum();
        assert_eq!(
            total_children,
            n - 1,
            "degree sum ({total_children}) must equal n-1 ({}) for a tree with {n} nodes",
            n - 1
        );

        let bit_len = 2 * n + 1;
        let num_words = bit_len.div_ceil(64);
        let mut bits = vec![0u64; num_words];

        // Sentinel super-root: bit 0 = 1, bit 1 = 0
        set_bit(&mut bits, 0);
        // bit 1 is already 0

        let mut pos = 2; // Start after sentinel "10"
        for &d in degrees {
            for _ in 0..d {
                set_bit(&mut bits, pos);
                pos += 1;
            }
            // Zero bit (separator) — already 0
            pos += 1;
        }

        assert_eq!(
            pos, bit_len,
            "encoding used {pos} bits but expected {bit_len}"
        );

        let rank_superblocks = build_rank_superblocks(&bits);

        Self {
            bits,
            rank_superblocks,
            bit_len,
            n,
        }
    }

    /// Build a LOUDS tree from a pointer-based tree (children list per node).
    ///
    /// `children[i]` is a slice of child node indices for node `i`. Nodes
    /// must be numbered `0..n` in BFS order.
    ///
    /// # Panics
    ///
    /// Panics if the tree structure is inconsistent.
    pub fn from_children(children: &[&[usize]]) -> Self {
        let degrees: Vec<usize> = children.iter().map(|c| c.len()).collect();
        Self::from_degrees(&degrees)
    }

    /// Number of nodes in the tree.
    #[inline]
    pub fn node_count(&self) -> usize {
        self.n
    }

    /// Whether the tree is empty (no nodes).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Total memory usage in bytes (excluding struct overhead).
    pub fn size_in_bytes(&self) -> usize {
        self.bits.len() * 8 + self.rank_superblocks.len() * 8
    }

    /// Degree (number of children) of node `v`.
    ///
    /// # Panics
    ///
    /// Panics if `v >= node_count()`.
    pub fn degree(&self, v: usize) -> usize {
        assert!(v < self.n, "node {v} out of bounds (n={})", self.n);
        // Node v's block ends at its 0-bit (select0(v+1)) and starts after
        // the previous node's 0-bit.  The degree is the number of 1-bits in
        // that block, which equals end - start.
        let end = self.select0(v + 1); // node v's terminating 0-bit
        let start = if v == 0 {
            self.select0(0) + 1 // after super-root sentinel "10"
        } else {
            self.select0(v) + 1 // after previous node's 0-bit
        };
        end - start
    }

    /// Parent of node `v`, or `None` for the root (node 0).
    ///
    /// # Panics
    ///
    /// Panics if `v >= node_count()`.
    pub fn parent(&self, v: usize) -> Option<usize> {
        assert!(v < self.n, "node {v} out of bounds (n={})", self.n);
        if v == 0 {
            return None;
        }
        // Node v (for v > 0) has a 1-bit in its parent's degree block.
        // The 0th 1-bit is the super-root sentinel, so node v's 1-bit is
        // the v-th 1-bit (0-indexed).
        let child_bit = self.select1(v);
        // rank0 counts 0-bits before this position. Subtract 1 for the
        // super-root's 0-bit to get the parent's BFS node index.
        Some(self.rank0(child_bit) - 1)
    }

    /// First child of node `v`, or `None` if `v` is a leaf.
    ///
    /// # Panics
    ///
    /// Panics if `v >= node_count()`.
    pub fn first_child(&self, v: usize) -> Option<usize> {
        assert!(v < self.n, "node {v} out of bounds (n={})", self.n);

        // Node v's degree block starts at select0(v) + 1 (after the v-th 0-bit).
        // If the first bit is 0, the node is a leaf.
        let block_start = self.select0(v) + 1;
        if block_start >= self.bit_len || !get_bit(&self.bits, block_start) {
            return None;
        }

        // The 1-bit AT block_start represents the first child.
        // rank1(block_start + 1) counts 1-bits in [0, block_start+1), including this one.
        // Subtract 1 for the super-root sentinel's 1-bit.
        Some(self.rank1(block_start + 1) - 1)
    }

    /// Next sibling of node `v`, or `None` if `v` is the last child.
    ///
    /// # Panics
    ///
    /// Panics if `v >= node_count()`.
    pub fn next_sibling(&self, v: usize) -> Option<usize> {
        assert!(v < self.n, "node {v} out of bounds (n={})", self.n);
        if v == 0 {
            return None; // root has no siblings
        }

        // Node v's 1-bit is at select1(v). The next bit is either another
        // 1-bit (next sibling) or a 0-bit (end of parent's degree block).
        let v_bit = self.select1(v);
        let next_bit = v_bit + 1;
        if next_bit >= self.bit_len || !get_bit(&self.bits, next_bit) {
            return None;
        }
        Some(v + 1)
    }

    /// Whether node `v` is a leaf (has no children).
    ///
    /// # Panics
    ///
    /// Panics if `v >= node_count()`.
    pub fn is_leaf(&self, v: usize) -> bool {
        self.first_child(v).is_none()
    }

    /// Depth of node `v` (root has depth 0).
    ///
    /// This is O(depth) — it walks to the root via `parent()`.
    ///
    /// # Panics
    ///
    /// Panics if `v >= node_count()`.
    pub fn depth(&self, v: usize) -> usize {
        let mut d = 0;
        let mut cur = v;
        while let Some(p) = self.parent(cur) {
            d += 1;
            cur = p;
        }
        d
    }

    /// Iterator over children of node `v`.
    ///
    /// # Panics
    ///
    /// Panics if `v >= node_count()`.
    pub fn children(&self, v: usize) -> ChildIter<'_> {
        assert!(v < self.n, "node {v} out of bounds (n={})", self.n);
        let first = self.first_child(v);
        ChildIter {
            tree: self,
            next: first,
        }
    }

    /// Subtree size rooted at node `v` (including `v` itself).
    ///
    /// This is O(subtree_size) — it performs a BFS within the subtree.
    ///
    /// # Panics
    ///
    /// Panics if `v >= node_count()`.
    pub fn subtree_size(&self, v: usize) -> usize {
        assert!(v < self.n, "node {v} out of bounds (n={})", self.n);
        let mut count = 0;
        let mut queue = vec![v];
        while let Some(node) = queue.pop() {
            count += 1;
            for child in self.children(node) {
                queue.push(child);
            }
        }
        count
    }

    // ── Bitvector primitives ────────────────────────────────────────

    /// Count 1-bits in `bits[0..pos)`.
    fn rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        let sb_idx = word_idx / SUPERBLOCK_WORDS;
        let mut count = self.rank_superblocks[sb_idx] as usize;

        let sb_start = sb_idx * SUPERBLOCK_WORDS;
        for i in sb_start..word_idx.min(self.bits.len()) {
            count += self.bits[i].count_ones() as usize;
        }

        if bit_idx > 0 && word_idx < self.bits.len() {
            let mask = (1u64 << bit_idx) - 1;
            count += (self.bits[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Count 0-bits in `bits[0..pos)`.
    fn rank0(&self, pos: usize) -> usize {
        pos - self.rank1(pos)
    }

    /// Find position of the `k`-th 1-bit (0-indexed).
    fn select1(&self, k: usize) -> usize {
        let target = k as u64;
        let mut lo = 0usize;
        let mut hi = self.rank_superblocks.len() - 1;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.rank_superblocks[mid + 1] <= target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        let sb = lo;
        let mut remaining = k - self.rank_superblocks[sb] as usize;
        let word_start = sb * SUPERBLOCK_WORDS;

        for w in word_start..self.bits.len() {
            let ones = self.bits[w].count_ones() as usize;
            if remaining < ones {
                return w * 64 + select_in_word(self.bits[w], remaining);
            }
            remaining -= ones;
        }

        panic!("select1({k}): not enough 1-bits")
    }

    /// Find position of the `k`-th 0-bit (0-indexed).
    fn select0(&self, k: usize) -> usize {
        let mut lo = 0usize;
        let mut hi = self.rank_superblocks.len() - 1;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let total_bits = (mid + 1) * SUPERBLOCK_WORDS * 64;
            let ones = self.rank_superblocks[mid + 1] as usize;
            let zeros = total_bits - ones;
            if zeros <= k {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        let sb = lo;
        let sb_total_bits = sb * SUPERBLOCK_WORDS * 64;
        let sb_ones = self.rank_superblocks[sb] as usize;
        let mut remaining = k - (sb_total_bits - sb_ones);
        let word_start = sb * SUPERBLOCK_WORDS;

        for w in word_start..self.bits.len() {
            let zeros = self.bits[w].count_zeros() as usize;
            if remaining < zeros {
                return w * 64 + select0_in_word(self.bits[w], remaining);
            }
            remaining -= zeros;
        }

        panic!("select0({k}): not enough 0-bits")
    }
}

/// Iterator over children of a node.
pub struct ChildIter<'a> {
    tree: &'a LoudsTree,
    next: Option<usize>,
}

impl Iterator for ChildIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        let v = self.next?;
        self.next = self.tree.next_sibling(v);
        Some(v)
    }
}

// ── Bit helpers ─────────────────────────────────────────────────────

/// Set bit at position `pos` in the bitvector.
fn set_bit(bits: &mut [u64], pos: usize) {
    let word = pos / 64;
    let bit = pos % 64;
    bits[word] |= 1u64 << bit;
}

/// Get bit at position `pos` in the bitvector.
fn get_bit(bits: &[u64], pos: usize) -> bool {
    let word = pos / 64;
    let bit = pos % 64;
    (bits[word] >> bit) & 1 == 1
}

/// Build rank superblocks for a bitvector.
fn build_rank_superblocks(bits: &[u64]) -> Vec<u64> {
    let num_superblocks = bits.len().div_ceil(SUPERBLOCK_WORDS);
    let mut superblocks = Vec::with_capacity(num_superblocks + 1);
    superblocks.push(0u64);
    let mut cumulative = 0u64;
    for chunk in bits.chunks(SUPERBLOCK_WORDS) {
        for &word in chunk {
            cumulative += word.count_ones() as u64;
        }
        superblocks.push(cumulative);
    }
    superblocks
}

/// Find position of the `k`-th 1-bit within a u64 word (0-indexed).
fn select_in_word(word: u64, k: usize) -> usize {
    let mut remaining = k;
    let mut w = word;
    for bit in 0..64 {
        if w & 1 == 1 {
            if remaining == 0 {
                return bit;
            }
            remaining -= 1;
        }
        w >>= 1;
        if w == 0 {
            break;
        }
    }
    unreachable!("select_in_word: not enough 1-bits")
}

/// Find position of the `k`-th 0-bit within a u64 word (0-indexed).
fn select0_in_word(word: u64, k: usize) -> usize {
    let mut remaining = k;
    let inverted = !word;
    let mut w = inverted;
    for bit in 0..64 {
        if w & 1 == 1 {
            if remaining == 0 {
                return bit;
            }
            remaining -= 1;
        }
        w >>= 1;
    }
    unreachable!("select0_in_word: not enough 0-bits")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Construction ───────────────────────────────────────────────

    #[test]
    fn single_node_tree() {
        let tree = LoudsTree::from_degrees(&[0]);
        assert_eq!(tree.node_count(), 1);
        assert!(tree.is_leaf(0));
        assert_eq!(tree.parent(0), None);
        assert_eq!(tree.first_child(0), None);
        assert_eq!(tree.depth(0), 0);
    }

    #[test]
    fn linear_chain() {
        // root -> a -> b -> c
        let tree = LoudsTree::from_degrees(&[1, 1, 1, 0]);
        assert_eq!(tree.node_count(), 4);

        assert_eq!(tree.parent(0), None);
        assert_eq!(tree.parent(1), Some(0));
        assert_eq!(tree.parent(2), Some(1));
        assert_eq!(tree.parent(3), Some(2));

        assert_eq!(tree.first_child(0), Some(1));
        assert_eq!(tree.first_child(1), Some(2));
        assert_eq!(tree.first_child(2), Some(3));
        assert!(tree.is_leaf(3));

        assert_eq!(tree.depth(0), 0);
        assert_eq!(tree.depth(1), 1);
        assert_eq!(tree.depth(2), 2);
        assert_eq!(tree.depth(3), 3);
    }

    #[test]
    fn binary_tree() {
        //       0
        //      / \
        //     1   2
        //    / \
        //   3   4
        let tree = LoudsTree::from_degrees(&[2, 2, 0, 0, 0]);
        assert_eq!(tree.node_count(), 5);

        assert_eq!(tree.first_child(0), Some(1));
        assert_eq!(tree.next_sibling(1), Some(2));
        assert_eq!(tree.next_sibling(2), None);

        assert_eq!(tree.first_child(1), Some(3));
        assert_eq!(tree.next_sibling(3), Some(4));
        assert!(tree.is_leaf(3));
        assert!(tree.is_leaf(4));
        assert!(tree.is_leaf(2));

        assert_eq!(tree.parent(1), Some(0));
        assert_eq!(tree.parent(2), Some(0));
        assert_eq!(tree.parent(3), Some(1));
        assert_eq!(tree.parent(4), Some(1));
    }

    #[test]
    fn wide_tree() {
        //       0
        //    / | | \
        //   1  2  3  4
        let tree = LoudsTree::from_degrees(&[4, 0, 0, 0, 0]);
        assert_eq!(tree.node_count(), 5);

        assert_eq!(tree.first_child(0), Some(1));
        assert_eq!(tree.next_sibling(1), Some(2));
        assert_eq!(tree.next_sibling(2), Some(3));
        assert_eq!(tree.next_sibling(3), Some(4));
        assert_eq!(tree.next_sibling(4), None);

        for i in 1..5 {
            assert!(tree.is_leaf(i));
            assert_eq!(tree.parent(i), Some(0));
            assert_eq!(tree.depth(i), 1);
        }
    }

    #[test]
    fn three_level_tree() {
        //       0
        //      / \
        //     1   2
        //    |   / \
        //    3  4   5
        let tree = LoudsTree::from_degrees(&[2, 1, 2, 0, 0, 0]);
        assert_eq!(tree.node_count(), 6);

        assert_eq!(tree.first_child(0), Some(1));
        assert_eq!(tree.next_sibling(1), Some(2));
        assert_eq!(tree.first_child(1), Some(3));
        assert_eq!(tree.first_child(2), Some(4));
        assert_eq!(tree.next_sibling(4), Some(5));

        assert_eq!(tree.parent(3), Some(1));
        assert_eq!(tree.parent(4), Some(2));
        assert_eq!(tree.parent(5), Some(2));

        assert_eq!(tree.depth(4), 2);
        assert_eq!(tree.depth(5), 2);
    }

    // ── Degree ────────────────────────────────────────────────────

    #[test]
    fn degree_matches_input() {
        let degrees = [2, 1, 2, 0, 0, 0];
        let tree = LoudsTree::from_degrees(&degrees);
        for (i, &d) in degrees.iter().enumerate() {
            assert_eq!(tree.degree(i), d, "degree mismatch at node {i}");
        }
    }

    // ── Children iterator ─────────────────────────────────────────

    #[test]
    fn children_iter() {
        let tree = LoudsTree::from_degrees(&[3, 0, 1, 0, 0]);
        let root_children: Vec<_> = tree.children(0).collect();
        assert_eq!(root_children, vec![1, 2, 3]);

        let node2_children: Vec<_> = tree.children(2).collect();
        assert_eq!(node2_children, vec![4]);

        let leaf_children: Vec<_> = tree.children(1).collect();
        assert!(leaf_children.is_empty());
    }

    // ── Subtree size ──────────────────────────────────────────────

    #[test]
    fn subtree_size_root() {
        let tree = LoudsTree::from_degrees(&[2, 2, 0, 0, 0]);
        assert_eq!(tree.subtree_size(0), 5);
    }

    #[test]
    fn subtree_size_leaf() {
        let tree = LoudsTree::from_degrees(&[2, 2, 0, 0, 0]);
        assert_eq!(tree.subtree_size(3), 1);
    }

    #[test]
    fn subtree_size_internal() {
        let tree = LoudsTree::from_degrees(&[2, 2, 0, 0, 0]);
        // Node 1 has children 3, 4
        assert_eq!(tree.subtree_size(1), 3);
    }

    // ── from_children ─────────────────────────────────────────────

    #[test]
    fn from_children_matches_degrees() {
        let children: &[&[usize]] = &[&[1, 2], &[3, 4], &[], &[], &[]];
        let tree = LoudsTree::from_children(children);
        assert_eq!(tree.node_count(), 5);
        assert_eq!(tree.first_child(0), Some(1));
        assert_eq!(tree.first_child(1), Some(3));
    }

    // ── Memory efficiency ─────────────────────────────────────────

    #[test]
    fn memory_much_less_than_pointers() {
        let n = 1000;
        // Complete binary tree with n leaves (2n-1 nodes total)
        // Build degree sequence in BFS order
        let total = 2 * n - 1;
        let mut degrees = vec![0usize; total];
        for d in degrees.iter_mut().take(n - 1) {
            *d = 2;
        }
        let tree = LoudsTree::from_degrees(&degrees);

        let louds_bytes = tree.size_in_bytes();
        let pointer_bytes = total * 3 * 8; // parent + first_child + next_sibling pointers
        assert!(
            louds_bytes < pointer_bytes / 10,
            "LOUDS ({louds_bytes}B) should be < 10% of pointer tree ({pointer_bytes}B)"
        );
    }

    #[test]
    fn memory_scaling() {
        for &n in &[100, 1000, 10_000] {
            let total = 2 * n - 1;
            let mut degrees = vec![0usize; total];
            for d in degrees.iter_mut().take(n - 1) {
                *d = 2;
            }
            let tree = LoudsTree::from_degrees(&degrees);
            let bits_per_node = (tree.size_in_bytes() * 8) as f64 / total as f64;
            // LOUDS uses ~2 bits per node plus superblock overhead
            assert!(
                bits_per_node < 4.0,
                "n={n}: {bits_per_node:.1} bits/node exceeds 4.0"
            );
        }
    }

    // ── Edge cases ────────────────────────────────────────────────

    #[test]
    fn root_no_siblings() {
        let tree = LoudsTree::from_degrees(&[2, 0, 0]);
        assert_eq!(tree.next_sibling(0), None);
    }

    #[test]
    #[should_panic(expected = "degree sum")]
    fn invalid_degree_sum_panics() {
        LoudsTree::from_degrees(&[3, 0, 0]); // sum=3, n-1=2
    }

    #[test]
    #[should_panic(expected = "must not be empty")]
    fn empty_degrees_panics() {
        LoudsTree::from_degrees(&[]);
    }

    // ── Property: parent-child consistency ────────────────────────

    #[test]
    fn parent_child_roundtrip() {
        // For every non-root node, parent(first_child(parent(v))) or a sibling == v
        let tree = LoudsTree::from_degrees(&[3, 2, 0, 1, 0, 0, 0]);
        for v in 1..tree.node_count() {
            let p = tree.parent(v).unwrap();
            // v should be reachable from p's children
            let children: Vec<_> = tree.children(p).collect();
            assert!(
                children.contains(&v),
                "node {v}'s parent is {p} but {v} not in parent's children: {children:?}"
            );
        }
    }

    #[test]
    fn all_nodes_reachable_from_root() {
        let tree = LoudsTree::from_degrees(&[3, 2, 0, 1, 0, 0, 0]);
        let mut visited = vec![false; tree.node_count()];
        let mut stack = vec![0usize];
        while let Some(v) = stack.pop() {
            visited[v] = true;
            for child in tree.children(v) {
                stack.push(child);
            }
        }
        assert!(visited.iter().all(|&v| v), "not all nodes reachable");
    }

    // ── Proptest ──────────────────────────────────────────────────

    #[cfg(test)]
    mod proptests {
        use super::*;
        use proptest::prelude::*;

        /// Generate a valid BFS degree sequence for a tree with n nodes.
        ///
        /// In a valid BFS sequence, after processing node i, the cumulative
        /// child count must be at least i+1 (so that node i+1 exists).
        fn arb_degree_sequence(max_nodes: usize) -> impl Strategy<Value = Vec<usize>> {
            (2..=max_nodes).prop_flat_map(|n| {
                prop::collection::vec(0..=4usize, n).prop_map(move |raw| {
                    let mut degrees = vec![0usize; n];
                    let mut total_children: usize = 0;
                    let target = n - 1;

                    for i in 0..n {
                        let remaining = target - total_children;
                        if remaining == 0 {
                            break;
                        }
                        // Must generate enough children so that node i+1 exists.
                        // After assigning degree to node i, cumulative children
                        // must be >= i + 1.
                        let min_needed = (i + 1).saturating_sub(total_children);
                        let max_allowed = remaining.min(raw[i].max(min_needed));
                        let d = max_allowed.max(min_needed);
                        degrees[i] = d;
                        total_children += d;
                    }

                    // If we still haven't assigned enough children, add to the
                    // first node that can absorb them.
                    let deficit = target.saturating_sub(total_children);
                    if deficit > 0 {
                        degrees[0] += deficit;
                    }

                    degrees
                })
            })
        }

        proptest! {
            #[test]
            fn navigation_consistent(degrees in arb_degree_sequence(50)) {
                let tree = LoudsTree::from_degrees(&degrees);
                let n = tree.node_count();
                prop_assert_eq!(n, degrees.len());

                // Every non-root has a parent
                for v in 1..n {
                    let p = tree.parent(v);
                    prop_assert!(p.is_some(), "node {v} has no parent");
                    prop_assert!(p.unwrap() < v, "parent {} >= child {v}", p.unwrap());
                }

                // Parent-child consistency
                for v in 0..n {
                    for child in tree.children(v) {
                        prop_assert_eq!(tree.parent(child), Some(v));
                    }
                }

                // Degree matches children count
                for (v, &expected_deg) in degrees.iter().enumerate().take(n) {
                    let child_count = tree.children(v).count();
                    prop_assert_eq!(tree.degree(v), child_count);
                    prop_assert_eq!(tree.degree(v), expected_deg);
                }
            }

            #[test]
            fn subtree_sizes_sum(degrees in arb_degree_sequence(30)) {
                let tree = LoudsTree::from_degrees(&degrees);
                // Root subtree = entire tree
                prop_assert_eq!(tree.subtree_size(0), tree.node_count());
            }

            #[test]
            fn depth_matches_parent_chain(degrees in arb_degree_sequence(50)) {
                let tree = LoudsTree::from_degrees(&degrees);
                for v in 0..tree.node_count() {
                    let d = tree.depth(v);
                    if v == 0 {
                        prop_assert_eq!(d, 0);
                    } else {
                        let parent_depth = tree.depth(tree.parent(v).unwrap());
                        prop_assert_eq!(d, parent_depth + 1);
                    }
                }
            }

            #[test]
            fn memory_sublinear(n in 50..500usize) {
                let total = 2 * n - 1;
                let mut degrees = vec![0usize; total];
                for d in degrees.iter_mut().take(n - 1) {
                    *d = 2;
                }
                let tree = LoudsTree::from_degrees(&degrees);
                let bits_per_node = (tree.size_in_bytes() * 8) as f64 / total as f64;
                // ~2 bits/node + superblock overhead; 5 bits is generous
                prop_assert!(bits_per_node < 5.0,
                    "n={n}: {bits_per_node:.1} bits/node exceeds 5.0");
            }
        }
    }
}
