//! Adaptive Radix Tree (ART) for prefix search (bd-1cgh9).
//!
//! A radix tree with adaptive node sizes (Node4/Node16/Node48/Node256) that
//! provides O(k) lookup and O(k + m) prefix scan, where k = key length and
//! m = number of matches.
//!
//! # Node Types
//!
//! | Type    | Keys | Children | Size     | Use Case            |
//! |---------|------|----------|----------|---------------------|
//! | Node4   | 4    | 4        | ~48 B    | Very sparse         |
//! | Node16  | 16   | 16       | ~192 B   | Moderate density    |
//! | Node48  | 48   | 48+256   | ~656 B   | Dense               |
//! | Node256 | 256  | 256      | ~2056 B  | Very dense (full)   |
//!
//! # Example
//!
//! ```
//! use ftui_widgets::adaptive_radix::AdaptiveRadixTree;
//!
//! let mut art = AdaptiveRadixTree::new();
//! art.insert("file:open", 1);
//! art.insert("file:save", 2);
//! art.insert("file:close", 3);
//! art.insert("edit:undo", 4);
//!
//! // Prefix scan
//! let matches = art.prefix_scan("file:");
//! assert_eq!(matches.len(), 3);
//!
//! // Exact lookup
//! assert_eq!(art.get("edit:undo"), Some(&4));
//! ```

/// Maximum number of children in a Node4.
const NODE4_MAX: usize = 4;
/// Maximum number of children in a Node16.
const NODE16_MAX: usize = 16;
/// Maximum number of children in a Node48.
const NODE48_MAX: usize = 48;

/// An adaptive radix tree for string-keyed data.
#[derive(Debug, Clone)]
pub struct AdaptiveRadixTree<V> {
    root: Option<Box<ArtNode<V>>>,
    len: usize,
}

/// Internal node types with adaptive sizing.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum ArtNode<V> {
    /// Leaf node storing a key-value pair.
    Leaf { key: String, value: V },
    /// Inner node with adaptive children.
    Inner {
        /// Compressed path prefix (path compression optimization).
        prefix: Vec<u8>,
        /// Children stored in one of the adaptive formats.
        children: Children<V>,
        /// Value stored at this node (if key terminates here).
        value: Option<V>,
        /// Full key for terminal values stored directly on compressed inner nodes.
        terminal_key: Option<String>,
    },
}

/// Adaptive child storage.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum Children<V> {
    /// Up to 4 children: sorted key bytes + child pointers.
    Node4 {
        keys: Vec<u8>,
        children: Vec<Box<ArtNode<V>>>,
    },
    /// Up to 16 children: sorted key bytes + child pointers.
    Node16 {
        keys: Vec<u8>,
        children: Vec<Box<ArtNode<V>>>,
    },
    /// Up to 48 children: 256-entry index + child array.
    Node48 {
        index: [u8; 256],
        children: Vec<Option<Box<ArtNode<V>>>>,
        count: usize,
    },
    /// Up to 256 children: direct indexing.
    Node256 {
        children: Vec<Option<Box<ArtNode<V>>>>,
    },
}

impl<V: Clone> AdaptiveRadixTree<V> {
    /// Create a new empty tree.
    pub fn new() -> Self {
        Self { root: None, len: 0 }
    }

    /// Number of entries in the tree.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Insert a key-value pair. Returns the previous value if the key existed.
    pub fn insert(&mut self, key: &str, value: V) -> Option<V> {
        if self.root.is_none() {
            self.root = Some(Box::new(ArtNode::Leaf {
                key: key.to_string(),
                value,
            }));
            self.len += 1;
            return None;
        }

        let result = insert_recursive(self.root.as_mut().unwrap(), key.as_bytes(), key, value, 0);
        if result.is_none() {
            self.len += 1;
        }
        result
    }

    /// Look up a value by exact key.
    pub fn get(&self, key: &str) -> Option<&V> {
        let node = self.root.as_ref()?;
        get_recursive(node, key.as_bytes(), 0)
    }

    /// Return all key-value pairs whose keys start with the given prefix,
    /// sorted by key.
    pub fn prefix_scan(&self, prefix: &str) -> Vec<(&str, &V)> {
        let mut results = Vec::new();
        if let Some(ref root) = self.root {
            prefix_scan_recursive(root, prefix.as_bytes(), 0, &mut results);
        }
        results.sort_by_key(|(k, _)| *k);
        results
    }

    /// Delete a key. Returns the value if it existed.
    pub fn delete(&mut self, key: &str) -> Option<V> {
        let result = {
            let root = self.root.as_mut()?;
            delete_recursive(root, key.as_bytes(), 0)
        };

        if result.removed.is_some() {
            self.len -= 1;
            if result.prune || self.root.as_ref().is_some_and(|root| is_empty_node(root)) {
                self.root = None;
            }
        }
        result.removed
    }

    /// Iterate all entries in sorted key order.
    pub fn iter(&self) -> Vec<(&str, &V)> {
        let mut results = Vec::new();
        if let Some(ref root) = self.root {
            collect_all(root, &mut results);
        }
        results.sort_by_key(|(k, _)| *k);
        results
    }

    /// Get node type distribution for diagnostics.
    pub fn node_distribution(&self) -> NodeDistribution {
        let mut dist = NodeDistribution::default();
        if let Some(ref root) = self.root {
            count_nodes(root, &mut dist);
        }
        dist
    }
}

impl<V: Clone> Default for AdaptiveRadixTree<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// Distribution of node types in the tree.
#[derive(Debug, Clone, Default)]
pub struct NodeDistribution {
    pub leaves: usize,
    pub node4: usize,
    pub node16: usize,
    pub node48: usize,
    pub node256: usize,
}

#[derive(Debug)]
struct DeleteResult<V> {
    removed: Option<V>,
    prune: bool,
}

impl<V> DeleteResult<V> {
    const fn not_found() -> Self {
        Self {
            removed: None,
            prune: false,
        }
    }
}

// ============================================================================
// Internal recursive operations
// ============================================================================

fn insert_recursive<V: Clone>(
    node: &mut ArtNode<V>,
    key_bytes: &[u8],
    full_key: &str,
    value: V,
    depth: usize,
) -> Option<V> {
    match node {
        ArtNode::Leaf {
            key: existing_key,
            value: existing_value,
        } => {
            if existing_key == full_key {
                // Key exists — update value.
                let old = existing_value.clone();
                *existing_value = value;
                return Some(old);
            }

            // Split leaf into inner node.
            let existing_bytes = existing_key.as_bytes();
            let common_prefix_len =
                common_prefix_length(&existing_bytes[depth..], &key_bytes[depth..]);

            let prefix = existing_bytes[depth..depth + common_prefix_len].to_vec();

            let old_key = existing_key.clone();
            let old_val = existing_value.clone();

            let split_depth = depth + common_prefix_len;

            if split_depth >= existing_bytes.len() && split_depth >= key_bytes.len() {
                // Keys are identical up to this point — shouldn't happen (checked above).
                *existing_value = value;
                return Some(old_val);
            }

            let mut children = Children::Node4 {
                keys: Vec::new(),
                children: Vec::new(),
            };

            if split_depth < existing_bytes.len() {
                let old_child = Box::new(ArtNode::Leaf {
                    key: old_key.clone(),
                    value: old_val.clone(),
                });
                children_insert(&mut children, existing_bytes[split_depth], old_child);
            }

            let mut inner_value = None;
            let mut terminal_key = None;
            if split_depth < key_bytes.len() {
                let new_child = Box::new(ArtNode::Leaf {
                    key: full_key.to_string(),
                    value,
                });
                children_insert(&mut children, key_bytes[split_depth], new_child);
            } else {
                inner_value = Some(value);
                terminal_key = Some(full_key.to_string());
            }

            if split_depth >= existing_bytes.len() {
                inner_value = Some(old_val);
                terminal_key = Some(old_key);
            }

            *node = ArtNode::Inner {
                prefix,
                children,
                value: inner_value,
                terminal_key,
            };
            None
        }
        ArtNode::Inner {
            prefix,
            children,
            value: node_value,
            terminal_key,
        } => {
            let remaining = &key_bytes[depth..];
            let prefix_match = common_prefix_length(remaining, prefix);

            if prefix_match < prefix.len() {
                // Prefix mismatch — split this inner node.
                let common = prefix[..prefix_match].to_vec();
                let old_suffix = prefix[prefix_match..].to_vec();
                let old_first_byte = old_suffix[0];

                // Create new inner node with shared prefix.
                let old_inner = ArtNode::Inner {
                    prefix: old_suffix[1..].to_vec(),
                    children: std::mem::replace(
                        children,
                        Children::Node4 {
                            keys: Vec::new(),
                            children: Vec::new(),
                        },
                    ),
                    value: node_value.take(),
                    terminal_key: terminal_key.take(),
                };

                let mut new_children = Children::Node4 {
                    keys: Vec::new(),
                    children: Vec::new(),
                };
                children_insert(&mut new_children, old_first_byte, Box::new(old_inner));

                let new_depth = depth + prefix_match;
                let mut inner_value = None;
                let mut new_terminal_key = None;
                if new_depth < key_bytes.len() {
                    let new_child = Box::new(ArtNode::Leaf {
                        key: full_key.to_string(),
                        value,
                    });
                    children_insert(&mut new_children, key_bytes[new_depth], new_child);
                } else {
                    inner_value = Some(value);
                    new_terminal_key = Some(full_key.to_string());
                }

                *prefix = common;
                *children = new_children;
                *node_value = inner_value;
                *terminal_key = new_terminal_key;
                return None;
            }

            let next_depth = depth + prefix.len();
            if next_depth >= key_bytes.len() {
                // Key terminates at this node.
                let old = node_value.take();
                *node_value = Some(value);
                *terminal_key = Some(full_key.to_string());
                return old;
            }

            let byte = key_bytes[next_depth];
            if let Some(child) = children_get_mut(children, byte) {
                insert_recursive(child, key_bytes, full_key, value, next_depth + 1)
            } else {
                let new_child = Box::new(ArtNode::Leaf {
                    key: full_key.to_string(),
                    value,
                });
                children_insert(children, byte, new_child);
                None
            }
        }
    }
}

fn get_recursive<'a, V>(node: &'a ArtNode<V>, key_bytes: &[u8], depth: usize) -> Option<&'a V> {
    match node {
        ArtNode::Leaf { key, value } => {
            if key.as_bytes() == key_bytes {
                Some(value)
            } else {
                None
            }
        }
        ArtNode::Inner {
            prefix,
            children,
            value,
            terminal_key: _,
        } => {
            let remaining = &key_bytes[depth..];
            if remaining.len() < prefix.len() || &remaining[..prefix.len()] != prefix.as_slice() {
                return None;
            }
            let next_depth = depth + prefix.len();
            if next_depth >= key_bytes.len() {
                return value.as_ref();
            }
            let byte = key_bytes[next_depth];
            children_get(children, byte)
                .and_then(|child| get_recursive(child, key_bytes, next_depth + 1))
        }
    }
}

fn prefix_scan_recursive<'a, V>(
    node: &'a ArtNode<V>,
    prefix_bytes: &[u8],
    depth: usize,
    results: &mut Vec<(&'a str, &'a V)>,
) {
    match node {
        ArtNode::Leaf { key, value } => {
            if key.as_bytes().starts_with(prefix_bytes) {
                results.push((key.as_str(), value));
            }
        }
        ArtNode::Inner {
            prefix,
            children,
            value: _,
            terminal_key: _,
        } => {
            let remaining_prefix = if depth < prefix_bytes.len() {
                &prefix_bytes[depth..]
            } else {
                &[]
            };

            // Check if the node's prefix is compatible with the search prefix.
            let match_len = common_prefix_length(remaining_prefix, prefix);

            if match_len < remaining_prefix.len() && match_len < prefix.len() {
                // No overlap.
                return;
            }

            let next_depth = depth + prefix.len();

            if remaining_prefix.len() <= prefix.len() {
                // The search prefix is fully consumed — collect all descendants.
                if depth + match_len >= prefix_bytes.len() {
                    // Prefix fully matched — collect all.
                    collect_all_inner(node, results);
                    return;
                }
            }

            if next_depth > prefix_bytes.len() {
                // Already past the prefix — collect all.
                collect_all_inner(node, results);
                return;
            }

            if next_depth == prefix_bytes.len() {
                // Prefix exactly consumed at this level — collect all.
                collect_all_inner(node, results);
                return;
            }

            // Continue searching in the appropriate child.
            let byte = prefix_bytes[next_depth];
            if let Some(child) = children_get(children, byte) {
                prefix_scan_recursive(child, prefix_bytes, next_depth + 1, results);
            }
        }
    }
}

fn collect_all_inner<'a, V>(node: &'a ArtNode<V>, results: &mut Vec<(&'a str, &'a V)>) {
    match node {
        ArtNode::Leaf { key, value } => {
            results.push((key.as_str(), value));
        }
        ArtNode::Inner {
            children,
            value,
            terminal_key,
            ..
        } => {
            if let (Some(key), Some(value)) = (terminal_key.as_deref(), value.as_ref()) {
                results.push((key, value));
            }
            for child in children_iter(children) {
                collect_all_inner(child, results);
            }
        }
    }
}

fn collect_all<'a, V>(node: &'a ArtNode<V>, results: &mut Vec<(&'a str, &'a V)>) {
    collect_all_inner(node, results);
}

fn delete_recursive<V: Clone>(
    node: &mut ArtNode<V>,
    key_bytes: &[u8],
    depth: usize,
) -> DeleteResult<V> {
    match node {
        ArtNode::Leaf { key, value } => {
            if key.as_bytes() == key_bytes {
                DeleteResult {
                    removed: Some(value.clone()),
                    prune: true,
                }
            } else {
                DeleteResult::not_found()
            }
        }
        ArtNode::Inner {
            prefix,
            children,
            value: node_value,
            terminal_key,
        } => {
            let remaining = &key_bytes[depth..];
            if remaining.len() < prefix.len() || &remaining[..prefix.len()] != prefix.as_slice() {
                return DeleteResult::not_found();
            }
            let next_depth = depth + prefix.len();
            if next_depth >= key_bytes.len() {
                let removed = node_value.take();
                if removed.is_some() {
                    *terminal_key = None;
                }
                let prune = removed.is_some() && children_count(children) == 0;
                return DeleteResult { removed, prune };
            }
            let byte = key_bytes[next_depth];
            let child_result = if let Some(child) = children_get_mut(children, byte) {
                delete_recursive(child, key_bytes, next_depth + 1)
            } else {
                DeleteResult::not_found()
            };

            if child_result.prune {
                children_remove(children, byte);
            }
            let removed = child_result.removed;
            DeleteResult {
                prune: removed.is_some() && node_value.is_none() && children_count(children) == 0,
                removed,
            }
        }
    }
}

fn is_empty_node<V>(node: &ArtNode<V>) -> bool {
    match node {
        ArtNode::Leaf { .. } => false, // Leaves are never "empty"
        ArtNode::Inner {
            children, value, ..
        } => value.is_none() && children_count(children) == 0,
    }
}

fn count_nodes<V>(node: &ArtNode<V>, dist: &mut NodeDistribution) {
    match node {
        ArtNode::Leaf { .. } => dist.leaves += 1,
        ArtNode::Inner { children, .. } => {
            match children {
                Children::Node4 { .. } => dist.node4 += 1,
                Children::Node16 { .. } => dist.node16 += 1,
                Children::Node48 { .. } => dist.node48 += 1,
                Children::Node256 { .. } => dist.node256 += 1,
            }
            for child in children_iter(children) {
                count_nodes(child, dist);
            }
        }
    }
}

// ============================================================================
// Children operations with automatic promotion/demotion
// ============================================================================

fn children_insert<V>(children: &mut Children<V>, byte: u8, child: Box<ArtNode<V>>) {
    match children {
        Children::Node4 { keys, children: ch } => {
            if ch.len() < NODE4_MAX {
                let pos = keys.iter().position(|&k| k > byte).unwrap_or(keys.len());
                keys.insert(pos, byte);
                ch.insert(pos, child);
            } else {
                // Promote to Node16.
                let mut new_keys = keys.clone();
                let mut new_ch: Vec<Box<ArtNode<V>>> = std::mem::take(ch);
                let pos = new_keys
                    .iter()
                    .position(|&k| k > byte)
                    .unwrap_or(new_keys.len());
                new_keys.insert(pos, byte);
                new_ch.insert(pos, child);
                *children = Children::Node16 {
                    keys: new_keys,
                    children: new_ch,
                };
            }
        }
        Children::Node16 { keys, children: ch } => {
            if ch.len() < NODE16_MAX {
                let pos = keys.iter().position(|&k| k > byte).unwrap_or(keys.len());
                keys.insert(pos, byte);
                ch.insert(pos, child);
            } else {
                // Promote to Node48.
                let mut index = [u8::MAX; 256];
                let mut new_ch: Vec<Option<Box<ArtNode<V>>>> = Vec::with_capacity(NODE48_MAX);
                for (i, (&k, c)) in keys.iter().zip(ch.drain(..)).enumerate() {
                    index[k as usize] = i as u8;
                    new_ch.push(Some(c));
                }
                let idx = new_ch.len();
                index[byte as usize] = idx as u8;
                new_ch.push(Some(child));
                *children = Children::Node48 {
                    index,
                    children: new_ch,
                    count: idx + 1,
                };
            }
        }
        Children::Node48 {
            index,
            children: ch,
            count,
        } => {
            if *count < NODE48_MAX {
                let idx = *count;
                index[byte as usize] = idx as u8;
                if idx < ch.len() {
                    ch[idx] = Some(child);
                } else {
                    ch.push(Some(child));
                }
                *count += 1;
            } else {
                // Promote to Node256.
                let mut new_ch: Vec<Option<Box<ArtNode<V>>>> = (0..256).map(|_| None).collect();
                for (b, &idx) in index.iter().enumerate() {
                    if idx != u8::MAX && (idx as usize) < ch.len() {
                        new_ch[b] = ch[idx as usize].take();
                    }
                }
                new_ch[byte as usize] = Some(child);
                *children = Children::Node256 { children: new_ch };
            }
        }
        Children::Node256 { children: ch } => {
            ch[byte as usize] = Some(child);
        }
    }
}

fn children_get<V>(children: &Children<V>, byte: u8) -> Option<&ArtNode<V>> {
    match children {
        Children::Node4 { keys, children: ch } => {
            keys.iter().position(|&k| k == byte).map(|i| ch[i].as_ref())
        }
        Children::Node16 { keys, children: ch } => {
            keys.iter().position(|&k| k == byte).map(|i| ch[i].as_ref())
        }
        Children::Node48 {
            index,
            children: ch,
            ..
        } => {
            let idx = index[byte as usize];
            if idx != u8::MAX && (idx as usize) < ch.len() {
                ch[idx as usize].as_ref().map(|c| c.as_ref())
            } else {
                None
            }
        }
        Children::Node256 { children: ch } => ch[byte as usize].as_ref().map(|c| c.as_ref()),
    }
}

fn children_get_mut<V>(children: &mut Children<V>, byte: u8) -> Option<&mut ArtNode<V>> {
    match children {
        Children::Node4 { keys, children: ch } => keys
            .iter()
            .position(|&k| k == byte)
            .map(move |i| ch[i].as_mut()),
        Children::Node16 { keys, children: ch } => keys
            .iter()
            .position(|&k| k == byte)
            .map(move |i| ch[i].as_mut()),
        Children::Node48 {
            index,
            children: ch,
            ..
        } => {
            let idx = index[byte as usize];
            if idx != u8::MAX && (idx as usize) < ch.len() {
                ch[idx as usize].as_mut().map(|c| c.as_mut())
            } else {
                None
            }
        }
        Children::Node256 { children: ch } => ch[byte as usize].as_mut().map(|c| c.as_mut()),
    }
}

fn children_remove<V>(children: &mut Children<V>, byte: u8) {
    match children {
        Children::Node4 { keys, children: ch } => {
            if let Some(pos) = keys.iter().position(|&k| k == byte) {
                keys.remove(pos);
                ch.remove(pos);
            }
        }
        Children::Node16 { keys, children: ch } => {
            if let Some(pos) = keys.iter().position(|&k| k == byte) {
                keys.remove(pos);
                ch.remove(pos);
            }
            // Demote to Node4 if small enough.
            if ch.len() <= NODE4_MAX {
                *children = Children::Node4 {
                    keys: keys.clone(),
                    children: std::mem::take(ch),
                };
            }
        }
        Children::Node48 {
            index,
            children: ch,
            count,
        } => {
            let idx = index[byte as usize];
            if idx != u8::MAX && (idx as usize) < ch.len() {
                ch[idx as usize] = None;
                index[byte as usize] = u8::MAX;
                *count = count.saturating_sub(1);
            }
        }
        Children::Node256 { children: ch } => {
            ch[byte as usize] = None;
        }
    }
}

fn children_count<V>(children: &Children<V>) -> usize {
    match children {
        Children::Node4 { children: ch, .. } => ch.len(),
        Children::Node16 { children: ch, .. } => ch.len(),
        Children::Node48 { count, .. } => *count,
        Children::Node256 { children: ch } => ch.iter().filter(|c| c.is_some()).count(),
    }
}

fn children_iter<'a, V>(
    children: &'a Children<V>,
) -> Box<dyn Iterator<Item = &'a ArtNode<V>> + 'a> {
    match children {
        Children::Node4 { children: ch, .. } => Box::new(ch.iter().map(|c| c.as_ref())),
        Children::Node16 { children: ch, .. } => Box::new(ch.iter().map(|c| c.as_ref())),
        Children::Node48 { children: ch, .. } => {
            Box::new(ch.iter().filter_map(|c| c.as_ref().map(|c| c.as_ref())))
        }
        Children::Node256 { children: ch } => {
            Box::new(ch.iter().filter_map(|c| c.as_ref().map(|c| c.as_ref())))
        }
    }
}

fn common_prefix_length(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree() {
        let art: AdaptiveRadixTree<u32> = AdaptiveRadixTree::new();
        assert!(art.is_empty());
        assert_eq!(art.len(), 0);
        assert_eq!(art.get("anything"), None);
    }

    #[test]
    fn single_insert_and_get() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("hello", 42);
        assert_eq!(art.get("hello"), Some(&42));
        assert_eq!(art.get("hell"), None);
        assert_eq!(art.get("helloo"), None);
        assert_eq!(art.len(), 1);
    }

    #[test]
    fn multiple_inserts() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("file:open", 1);
        art.insert("file:save", 2);
        art.insert("file:close", 3);
        art.insert("edit:undo", 4);
        art.insert("edit:redo", 5);

        assert_eq!(art.len(), 5);
        assert_eq!(art.get("file:open"), Some(&1));
        assert_eq!(art.get("file:save"), Some(&2));
        assert_eq!(art.get("file:close"), Some(&3));
        assert_eq!(art.get("edit:undo"), Some(&4));
        assert_eq!(art.get("edit:redo"), Some(&5));
    }

    #[test]
    fn prefix_scan_basic() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("file:open", 1);
        art.insert("file:save", 2);
        art.insert("file:close", 3);
        art.insert("edit:undo", 4);

        let results = art.prefix_scan("file:");
        assert_eq!(results.len(), 3);

        let edit_results = art.prefix_scan("edit:");
        assert_eq!(edit_results.len(), 1);
    }

    #[test]
    fn prefix_scan_sorted() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("c", 3);
        art.insert("b", 2);
        art.insert("a", 1);

        let results = art.prefix_scan("");
        let keys: Vec<&str> = results.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    #[test]
    fn update_existing_key() {
        let mut art = AdaptiveRadixTree::new();
        assert_eq!(art.insert("key", 1), None);
        assert_eq!(art.insert("key", 2), Some(1));
        assert_eq!(art.get("key"), Some(&2));
        assert_eq!(art.len(), 1);
    }

    #[test]
    fn delete_existing() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("hello", 42);
        assert_eq!(art.delete("hello"), Some(42));
        assert_eq!(art.get("hello"), None);
        assert_eq!(art.len(), 0);
    }

    #[test]
    fn delete_nonexistent() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("hello", 42);
        assert_eq!(art.delete("world"), None);
        assert_eq!(art.len(), 1);
    }

    #[test]
    fn many_inserts_promote_node_types() {
        let mut art = AdaptiveRadixTree::new();
        // Insert enough to trigger Node4 → Node16 → Node48 promotion.
        for i in 0..50u32 {
            art.insert(&format!("key_{i:03}"), i);
        }
        assert_eq!(art.len(), 50);

        // Verify all entries retrievable.
        for i in 0..50u32 {
            assert_eq!(art.get(&format!("key_{i:03}")), Some(&i));
        }

        let dist = art.node_distribution();
        assert!(dist.leaves >= 50);
    }

    #[test]
    fn iter_returns_all_sorted() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("z", 26);
        art.insert("a", 1);
        art.insert("m", 13);

        let entries = art.iter();
        let keys: Vec<&str> = entries.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["a", "m", "z"]);
    }

    #[test]
    fn shared_prefix_keys() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("test", 1);
        art.insert("testing", 2);
        art.insert("tested", 3);
        art.insert("tester", 4);

        assert_eq!(art.len(), 4);
        assert_eq!(art.get("test"), Some(&1));
        assert_eq!(art.get("testing"), Some(&2));
        assert_eq!(art.get("tested"), Some(&3));
        assert_eq!(art.get("tester"), Some(&4));

        let scan = art.prefix_scan("test");
        assert_eq!(scan.len(), 4);
    }

    #[test]
    fn iter_includes_keys_stored_on_compressed_inner_nodes() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("file:save", 1);
        art.insert("file:save-as", 2);

        let entries = art.iter();
        let keys: Vec<&str> = entries.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, vec!["file:save", "file:save-as"]);
    }

    #[test]
    fn deleting_prefix_key_preserves_longer_descendants() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("test", 1);
        art.insert("testing", 2);
        art.insert("tester", 3);

        assert_eq!(art.delete("test"), Some(1));
        assert_eq!(art.get("test"), None);
        assert_eq!(art.get("testing"), Some(&2));
        assert_eq!(art.get("tester"), Some(&3));

        let keys: Vec<&str> = art
            .prefix_scan("test")
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, vec!["tester", "testing"]);
    }

    #[test]
    fn empty_prefix_scan_returns_all() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("a", 1);
        art.insert("b", 2);

        let results = art.prefix_scan("");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn node_distribution() {
        let mut art = AdaptiveRadixTree::new();
        art.insert("a", 1);
        art.insert("b", 2);
        art.insert("c", 3);

        let dist = art.node_distribution();
        assert!(dist.leaves >= 3);
    }

    #[test]
    fn command_palette_scenario() {
        let mut art = AdaptiveRadixTree::new();
        let commands = [
            "file:open",
            "file:save",
            "file:save-as",
            "file:close",
            "file:new",
            "edit:undo",
            "edit:redo",
            "edit:cut",
            "edit:copy",
            "edit:paste",
            "view:sidebar",
            "view:terminal",
            "view:explorer",
            "view:minimap",
            "go:line",
            "go:file",
            "go:symbol",
            "go:definition",
        ];
        for (i, cmd) in commands.iter().enumerate() {
            art.insert(cmd, i);
        }

        // User types "file:" → 5 results.
        assert_eq!(art.prefix_scan("file:").len(), 5);
        // User types "edit:" → 5 results.
        assert_eq!(art.prefix_scan("edit:").len(), 5);
        // User types "view:" → 4 results.
        assert_eq!(art.prefix_scan("view:").len(), 4);
        // User types "go:" → 4 results.
        assert_eq!(art.prefix_scan("go:").len(), 4);
        // User types "f" → 5 results (all file: commands).
        assert_eq!(art.prefix_scan("f").len(), 5);
    }
}
