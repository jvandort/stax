use wasm_bindgen::prelude::*;

use crate::{get_children, Error, GraphBuilder, Names, WasmStackGraph};

/// Merges all occurrences of `node_name` into a single aggregate root.
#[wasm_bindgen]
pub fn wasm_merge_children(
    children_offsets: &[i32],
    children_data: &[i32],
    name_ids: &[i32],
    names_data: &[u8],
    names_offsets: &[i32],
    values: &[i64],
    node_name: &str,
) -> Result<WasmStackGraph, Error> {
    let names = Names::new(name_ids, names_data, names_offsets);
    let size = values.len();
    let target = names.find(node_name);
    // In node order, so the merged graph's node ids are the same on every run.
    let root_nodes: Vec<usize> = (0..size).filter(|&i| Some(names.id(i)) == target).collect();
    let mut is_root = vec![false; size];
    for &root in &root_nodes {
        is_root[root] = true;
    }

    if root_nodes.is_empty() {
        return Err(Error::NoSuchName(node_name.to_string()));
    }

    let mut new_graph = GraphBuilder::new(node_name);
    let mut id_map = vec![u32::MAX; names.string_count()];

    // Tracks the parent of the ith node in new_graph so we can walk ancestors
    // when subtracting double-counted samples (see below). Index 0 is the root.
    let mut new_parents: Vec<i32> = vec![-1i32];

    for &root_node in &root_nodes {
        let mut stack: Vec<(usize, usize)> = Vec::new();

        for &child in get_children(children_offsets, children_data, root_node) {
            stack.push((child as usize, 0));
        }

        while let Some((current, new_parent)) = stack.pop() {
            if is_root[current] {
                // This is a recursive call back into the merged node. Its subtree
                // will be attributed when we process it as a root_node in the outer
                // loop. Subtract its inclusive value from all ancestors in new_graph
                // now to avoid double-counting those samples.
                let mut ancestor = new_parent as i32;
                while ancestor >= 0 {
                    new_graph.values[ancestor as usize] -= values[current];
                    ancestor = new_parents[ancestor as usize];
                }
                continue;
            }

            let (new_current, created) =
                new_graph.get_or_create_child_from(new_parent, &names, current, &mut id_map, values[current]);
            if created {
                new_parents.push(new_parent as i32);
            }

            for &child in get_children(children_offsets, children_data, current) {
                stack.push((child as usize, new_current));
            }
        }
    }

    for &root_node in &root_nodes {
        new_graph.values[0] += values[root_node];
    }

    // A path that only leads back into a nested call of the merged node is left with no samples
    // (they were attributed to that nested call), so it carries no information.
    new_graph.remove_empty_nodes();
    Ok(new_graph.into_wasm_stack_graph())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_stacks_impl;
    use crate::testing::assert_valid_graph;

    fn merge(stacks: &[u8], name: &str) -> Result<WasmStackGraph, Error> {
        let g = parse_stacks_impl(stacks);
        let merged = wasm_merge_children(
            &g.children_offsets,
            &g.children_data,
            &g.name_ids,
            &g.names_data,
            &g.names_offsets,
            &g.values,
            name,
        )?;
        assert_valid_graph(&merged);
        Ok(merged)
    }

    /// `name;child;... value` lines for every node, root-first, to compare graphs by content.
    fn dump(g: &WasmStackGraph) -> Vec<String> {
        let mut lines = Vec::new();
        let mut stack = vec![(0usize, g.names().of(0).to_string())];
        while let Some((node, path)) = stack.pop() {
            lines.push(format!("{path} {}", g.values[node]));
            for &c in get_children(&g.children_offsets, &g.children_data, node) {
                stack.push((c as usize, format!("{path};{}", g.names().of(c as usize))));
            }
        }
        lines.sort();
        lines
    }

    #[test]
    fn test_merges_all_call_sites() {
        let merged = merge(b"a;f;x 3\nb;f;x 2\nb;f;y 1\n", "f").unwrap();
        assert_eq!(dump(&merged), ["f 6", "f;x 5", "f;y 1"]);
    }

    #[test]
    fn test_recursive_calls_count_once() {
        // The merged root counts each stack once. A nested f's samples go to that f's own subtree
        // and are subtracted from the outer f's, so f;g keeps no samples here (its only stack
        // re-enters f) and is dropped; a g with work of its own stays.
        let merged = merge(b"a;f;g;f;x 4\nf 1\n", "f").unwrap();
        assert_eq!(merged.values[0], 5);
        assert_eq!(dump(&merged), ["f 5", "f;x 4"]);
        let merged = merge(b"a;f;g;f;x 4\na;f;g;y 2\n", "f").unwrap();
        assert_eq!(dump(&merged), ["f 6", "f;g 2", "f;g;y 2", "f;x 4"]);
    }

    #[test]
    fn test_unknown_name_is_an_error() {
        assert!(matches!(merge(b"a;b 1\n", "zzz"), Err(Error::NoSuchName(_))));
    }

    #[test]
    fn test_node_ids_follow_the_source_graph_order() {
        // Many call sites, so a hash-ordered walk would almost surely reorder them.
        let stacks: String = (0..200).map(|i| format!("caller{i};f;callee{i} 1\n")).collect();
        let merged = merge(stacks.as_bytes(), "f").unwrap();
        let callees: Vec<&str> = (1..merged.node_count() as usize).map(|i| merged.names().of(i)).collect();
        let g = parse_stacks_impl(stacks.as_bytes());
        let expected: Vec<&str> =
            (0..g.node_count() as usize).map(|i| g.names().of(i)).filter(|n| n.starts_with("callee")).collect();
        assert_eq!(callees, expected);
    }
}
