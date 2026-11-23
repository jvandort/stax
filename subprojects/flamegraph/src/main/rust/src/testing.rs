//! Test helpers shared by the graph operations' tests.

use ahash::AHashSet;

use crate::{get_children, WasmStackGraph};

/// Checks the invariants every graph an operation produces must hold:
/// - the per-node arrays agree on the node count, and the CSR offsets are well-formed;
/// - every node's name id is in the string table, whose strings are distinct, and the display
///   names are indexed by the same ids;
/// - each node is the child of at most one node, never of itself or the root's child list's
///   owner twice (deleted nodes are left as unreachable orphans, so not every node is a child);
/// - children are sorted by name, with no two siblings sharing a name;
/// - a node's value covers its children's.
#[track_caller]
pub(crate) fn assert_valid_graph(g: &WasmStackGraph) {
    let n = g.name_ids.len();
    assert!(n >= 1, "a graph has at least a root");
    assert_eq!(g.values.len(), n, "values per node");
    assert_eq!(g.children_offsets.len(), n + 1, "children offsets per node");
    assert_eq!(g.children_offsets[0], 0);
    assert!(g.children_offsets.windows(2).all(|w| w[0] <= w[1]), "children offsets ascending");
    assert_eq!(*g.children_offsets.last().unwrap() as usize, g.children_data.len());

    let names = g.names();
    let strings = names.string_count();
    assert!(g.name_ids.iter().all(|&id| id >= 0 && (id as usize) < strings), "name ids in range");
    let distinct: AHashSet<&str> = (0..strings).map(|s| names.string(s)).collect();
    assert_eq!(distinct.len(), strings, "strings are distinct");
    assert_eq!(g.display_names_offsets.len(), g.names_offsets.len(), "a display name per string");

    let mut is_child = vec![false; n];
    for i in 0..n {
        let children = get_children(&g.children_offsets, &g.children_data, i);
        for &c in children {
            let c = c as usize;
            assert!(c < n && c != 0 && c != i, "child {c} of {i} is a valid non-root node");
            assert!(!is_child[c], "node {c} has two parents");
            is_child[c] = true;
        }
        for pair in children.windows(2) {
            let (a, b) = (names.of(pair[0] as usize), names.of(pair[1] as usize));
            assert!(a < b, "children of {i} sorted by name with no duplicates: {a:?} before {b:?}");
        }
        let child_sum: i64 = children.iter().map(|&c| g.values[c as usize]).sum();
        assert!(g.values[i] >= child_sum, "node {i} ({}) value {} < children's {child_sum}", names.of(i), g.values[i]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_stacks_impl;

    #[test]
    #[should_panic(expected = "sorted by name")]
    fn test_rejects_unsorted_children() {
        let mut g = parse_stacks_impl(b"a 1\nb 1\n");
        g.children_data.swap(0, 1);
        assert_valid_graph(&g);
    }

    #[test]
    #[should_panic(expected = "two parents")]
    fn test_rejects_shared_children() {
        let mut g = parse_stacks_impl(b"a;c 1\nb 1\n");
        // Make b (a root child) also a's child, replacing c.
        let b = g.children_data[1];
        let a_children = g.children_offsets[g.children_data[0] as usize] as usize;
        g.children_data[a_children] = b;
        assert_valid_graph(&g);
    }

    #[test]
    #[should_panic(expected = "strings are distinct")]
    fn test_rejects_duplicate_strings() {
        let mut g = parse_stacks_impl(b"a 1\n");
        g.names_data.extend_from_slice(b"a");
        g.names_offsets.push(g.names_data.len() as i32);
        g.display_names_offsets.push(*g.display_names_offsets.last().unwrap());
        assert_valid_graph(&g);
    }

    #[test]
    #[should_panic(expected = "value")]
    fn test_rejects_children_outweighing_their_parent() {
        let mut g = parse_stacks_impl(b"a;b 1\n");
        g.values[2] = 5;
        assert_valid_graph(&g);
    }
}
