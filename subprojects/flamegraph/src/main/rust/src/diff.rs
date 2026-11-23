use std::borrow::Cow;

use wasm_bindgen::prelude::*;

use crate::simplify::wasm_simplify_graph;
use crate::{get_children, DiffBuilder, Names, WasmDiffGraph, WasmStackGraph};

/// Merges the children of `a_node` and `b_node` by name using a sorted merge, and pushes
/// `(name, a_child, b_child, new_parent)` entries onto `stack` for each unique child name.
/// Children that exist in only one graph get `None` for the missing side.
///
/// This is allocation-free since children are already sorted alphabetically by name
/// (guaranteed by `build_children_csr`).
fn push_matched_children<'a>(
    stack: &mut Vec<(&'a str, Option<usize>, Option<usize>, usize)>,
    a_node: Option<usize>,
    b_node: Option<usize>,
    new_parent: usize,
    a_children_offsets: &[i32],
    a_children_data: &[i32],
    a_names: &Names<'a>,
    b_children_offsets: &[i32],
    b_children_data: &[i32],
    b_names: &Names<'a>,
) {
    let a_children = a_node
        .map(|a| get_children(a_children_offsets, a_children_data, a))
        .unwrap_or(&[]);
    let b_children = b_node
        .map(|b| get_children(b_children_offsets, b_children_data, b))
        .unwrap_or(&[]);

    let mut ai = 0;
    let mut bi = 0;
    while ai < a_children.len() && bi < b_children.len() {
        let a_child = a_children[ai] as usize;
        let b_child = b_children[bi] as usize;
        let a_name = a_names.of(a_child);
        let b_name = b_names.of(b_child);
        match a_name.cmp(b_name) {
            std::cmp::Ordering::Equal => {
                stack.push((a_name, Some(a_child), Some(b_child), new_parent));
                ai += 1;
                bi += 1;
            }
            std::cmp::Ordering::Less => {
                stack.push((a_name, Some(a_child), None, new_parent));
                ai += 1;
            }
            std::cmp::Ordering::Greater => {
                stack.push((b_name, None, Some(b_child), new_parent));
                bi += 1;
            }
        }
    }
    for &a_child in &a_children[ai..] {
        stack.push((a_names.of(a_child as usize), Some(a_child as usize), None, new_parent));
    }
    for &b_child in &b_children[bi..] {
        stack.push((b_names.of(b_child as usize), None, Some(b_child as usize), new_parent));
    }
}

fn diff_simplified(a: &WasmStackGraph, b: &WasmStackGraph) -> WasmDiffGraph {
    let (a_names, b_names) = (a.names(), b.names());
    let a_root_name = a_names.of(0);
    let b_root_name = b_names.of(0);
    let root_name = if a_root_name == b_root_name {
        Cow::Borrowed(a_root_name)
    } else {
        Cow::Owned(format!("{} / {}", a_root_name, b_root_name))
    };

    let mut builder = DiffBuilder::new(&root_name);
    builder.values_a[0] = a.values[0];
    builder.values_b[0] = b.values[0];

    // Stack entries: (name, a_node, b_node, new_parent_id)
    let mut stack: Vec<(&str, Option<usize>, Option<usize>, usize)> = Vec::new();
    push_matched_children(
        &mut stack,
        Some(0), Some(0), 0,
        &a.children_offsets, &a.children_data, &a_names,
        &b.children_offsets, &b.children_data, &b_names,
    );

    while let Some((name, a_node, b_node, new_parent)) = stack.pop() {
        let value_a = a_node.map_or(0, |i| a.values[i]);
        let value_b = b_node.map_or(0, |i| b.values[i]);

        let (new_current, _) = builder.get_or_create_child(new_parent, name, value_a, value_b);

        push_matched_children(
            &mut stack,
            a_node, b_node, new_current,
            &a.children_offsets, &a.children_data, &a_names,
            &b.children_offsets, &b.children_data, &b_names,
        );
    }

    builder.build()
}

/// Simplifies both input graphs and aligns them by call path, producing a diff graph
/// carrying each side's inclusive sample counts (0 for call paths absent on that
/// side): graph A's in the inner graph's `values` array, graph B's in `b_values`.
/// All diff measures are derived from the two count arrays client-side.
///
/// Nodes that exist in both graphs are merged into a single node; nodes present
/// in only one graph appear with the other side's count as 0.
#[wasm_bindgen]
pub fn wasm_diff_graphs(
    a_children_offsets: &[i32],
    a_children_data: &[i32],
    a_name_ids: &[i32],
    a_names_data: &[u8],
    a_names_offsets: &[i32],
    a_values: &[i64],
    b_children_offsets: &[i32],
    b_children_data: &[i32],
    b_name_ids: &[i32],
    b_names_data: &[u8],
    b_names_offsets: &[i32],
    b_values: &[i64],
) -> WasmDiffGraph {
    let a = wasm_simplify_graph(a_children_offsets, a_children_data, a_name_ids, a_names_data, a_names_offsets, a_values, 0);
    let b = wasm_simplify_graph(b_children_offsets, b_children_data, b_name_ids, b_names_data, b_names_offsets, b_values, 0);
    diff_simplified(&a, &b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_stacks_impl;

    fn diff(a: &[u8], b: &[u8]) -> WasmDiffGraph {
        let ga = parse_stacks_impl(a);
        let gb = parse_stacks_impl(b);
        let g = wasm_diff_graphs(
            &ga.children_offsets, &ga.children_data, &ga.name_ids, &ga.names_data, &ga.names_offsets, &ga.values,
            &gb.children_offsets, &gb.children_data, &gb.name_ids, &gb.names_data, &gb.names_offsets, &gb.values,
        );
        crate::testing::assert_valid_graph(&g.graph);
        // B's counts must form a valid graph over the same nodes too.
        crate::testing::assert_valid_graph(&WasmStackGraph { values: g.b_values.clone(), ..g.graph.clone() });
        g
    }

    fn node_names(g: &WasmDiffGraph) -> Vec<String> {
        (0..g.graph.node_count() as usize).map(|i| g.graph.names().of(i).to_string()).collect()
    }

    #[test]
    fn test_identical_graphs_preserve_a_and_b_values() {
        let data = b"a;b 10\na;c 5\n";
        let g = diff(data, data);
        // Graph A's counts ride in the inner graph's values array.
        assert_eq!(g.graph.values, g.b_values);
    }

    #[test]
    fn test_node_only_in_a_has_zero_b_value() {
        let g = diff(b"a;b 10\n", b"a;c 5\n");
        let names = node_names(&g);
        let b_idx = names.iter().position(|n| n == "b").unwrap();
        assert_eq!(g.graph.values[b_idx], 10);
        assert_eq!(g.b_values[b_idx], 0);
    }

    #[test]
    fn test_node_only_in_b_has_zero_a_value() {
        let g = diff(b"a;b 10\n", b"a;c 5\n");
        let names = node_names(&g);
        let c_idx = names.iter().position(|n| n == "c").unwrap();
        assert_eq!(g.graph.values[c_idx], 0);
        assert_eq!(g.b_values[c_idx], 5);
    }

    #[test]
    fn test_shared_node_accumulates_both_values() {
        let g = diff(b"a;b 10\n", b"a;b 7\n");
        let names = node_names(&g);
        let b_idx = names.iter().position(|n| n == "b").unwrap();
        assert_eq!(g.graph.values[b_idx], 10);
        assert_eq!(g.b_values[b_idx], 7);
    }
}
