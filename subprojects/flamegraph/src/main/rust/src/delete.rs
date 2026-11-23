use wasm_bindgen::prelude::*;

use crate::{build_parent_map, get_children, Error, WasmStackGraph};

/// Deletes a node from the graph by severing it from its parent's children list
/// and subtracting its weight from all ancestors.
///
/// The deleted node and its entire subtree are left as unreachable orphans in the
/// name ids and values arrays — they are never visited during rendering,
/// so this is harmless and avoids a full graph compaction.
#[wasm_bindgen]
pub fn wasm_delete_node(
    children_offsets: &[i32],
    children_data: &[i32],
    name_ids: &[i32],
    names_data: &[u8],
    names_offsets: &[i32],
    values: &[i64],
    display_names_data: &[u8],
    display_names_offsets: &[i32],
    to_delete: u32,
) -> Result<WasmStackGraph, Error> {
    let to_delete = to_delete as usize;
    if to_delete == 0 {
        return Err(Error::DeleteRoot);
    }

    let size = name_ids.len();
    let parents = build_parent_map(children_offsets, children_data);

    let parent_id = parents[to_delete];
    if parent_id < 0 {
        return Err(Error::NoParent(to_delete));
    }

    // Subtract deleted node's weight from all ancestors
    let deleted_value = values[to_delete];
    let mut new_values = values.to_vec();
    let mut curr = parent_id;
    while curr >= 0 {
        new_values[curr as usize] -= deleted_value;
        curr = parents[curr as usize];
    }

    // Rebuild the children arrays with node_id removed from its parent's child list.
    // Node count is unchanged so new_children_offsets stays the same length.
    // new_children_data loses exactly one entry — the deleted node's id.
    let mut new_children_offsets: Vec<i32> = Vec::with_capacity(children_offsets.len());
    let mut new_children_data: Vec<i32> = Vec::with_capacity(children_data.len() - 1);

    for i in 0..size {
        new_children_offsets.push(new_children_data.len() as i32);
        let children = get_children(children_offsets, children_data, i);
        if i == parent_id as usize {
            for &child in children {
                if child as usize != to_delete {
                    new_children_data.push(child);
                }
            }
        } else {
            new_children_data.extend_from_slice(children);
        }
    }
    new_children_offsets.push(new_children_data.len() as i32);

    Ok(WasmStackGraph {
        children_offsets: new_children_offsets,
        children_data: new_children_data,
        name_ids: name_ids.to_vec(),
        names_data: names_data.to_vec(),
        names_offsets: names_offsets.to_vec(),
        display_names_data: display_names_data.to_vec(),
        display_names_offsets: display_names_offsets.to_vec(),
        values: new_values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_stacks_impl;
    use crate::testing::assert_valid_graph;

    fn delete(g: &WasmStackGraph, node: u32) -> Result<WasmStackGraph, Error> {
        wasm_delete_node(
            &g.children_offsets,
            &g.children_data,
            &g.name_ids,
            &g.names_data,
            &g.names_offsets,
            &g.values,
            &g.display_names_data,
            &g.display_names_offsets,
            node,
        )
    }

    fn find(g: &WasmStackGraph, name: &str) -> usize {
        (0..g.node_count() as usize).find(|&i| g.names().of(i) == name).unwrap()
    }

    #[test]
    fn test_delete_subtracts_from_ancestors_and_unlinks() {
        let g = parse_stacks_impl(b"a;b;c 5\na;b;d 3\na;e 2\n");
        let (a, b, d) = (find(&g, "a"), find(&g, "b"), find(&g, "d"));
        let deleted = delete(&g, d as u32).unwrap();
        assert_valid_graph(&deleted);
        assert_eq!((deleted.values[0], deleted.values[a], deleted.values[b]), (7, 7, 5));
        let b_children = get_children(&deleted.children_offsets, &deleted.children_data, b);
        assert_eq!(b_children.iter().map(|&c| deleted.names().of(c as usize)).collect::<Vec<_>>(), ["c"]);
        // Nodes keep their ids and names; the deleted one is left as an unreachable orphan.
        assert_eq!(deleted.name_ids, g.name_ids);
        assert_eq!(deleted.names().of(d), "d");
    }

    #[test]
    fn test_delete_subtree() {
        let g = parse_stacks_impl(b"a;b;c 5\na;e 2\n");
        let deleted = delete(&g, find(&g, "b") as u32).unwrap();
        assert_valid_graph(&deleted);
        assert_eq!(deleted.values[0], 2);
        assert_eq!(deleted.values[find(&g, "a")], 2);
    }

    #[test]
    fn test_delete_errors() {
        let g = parse_stacks_impl(b"a;b 1\n");
        assert!(matches!(delete(&g, 0), Err(Error::DeleteRoot)));
        // Deleting an already deleted node: it no longer has a parent.
        let b = find(&g, "b") as u32;
        let deleted = delete(&g, b).unwrap();
        assert!(matches!(delete(&deleted, b), Err(Error::NoParent(_))));
    }
}
