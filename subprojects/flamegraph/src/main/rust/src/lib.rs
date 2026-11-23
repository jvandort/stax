use wasm_bindgen::prelude::*;

mod builder;
pub(crate) use builder::{DiffBuilder, GraphBuilder};

mod error;
pub use error::Error;

mod diff;
pub use diff::wasm_diff_graphs;

mod callgraph;
pub use callgraph::CallGraph;

mod delete;
pub use delete::wasm_delete_node;

mod icicle;
pub use icicle::wasm_icicle_graph;

mod jfr;
pub use jfr::{JfrParser, WasmJfrGraphs};

mod merge;
pub use merge::wasm_merge_children;

mod parser;
pub use parser::StacksParser;
#[cfg(test)]
pub use parser::parse_stacks_impl;

mod simplify;
pub use simplify::wasm_simplify_graph;

#[cfg(test)]
mod testing;

#[wasm_bindgen(start)]
pub fn main_js() {
    #[cfg(debug_assertions)]
    console_error_panic_hook::set_once();
}

/// A graph's node names: the name of node `i` is string `ids[i]` of the string table.
#[derive(Clone, Copy)]
pub(crate) struct Names<'a> {
    ids: &'a [i32],
    data: &'a [u8],
    offsets: &'a [i32],
}

impl<'a> Names<'a> {
    pub(crate) fn new(ids: &'a [i32], data: &'a [u8], offsets: &'a [i32]) -> Self {
        Names { ids, data, offsets }
    }

    /// The name of node `i`.
    #[inline]
    pub(crate) fn of(&self, i: usize) -> &'a str {
        self.string(self.ids[i] as usize)
    }

    /// The name id of node `i`.
    #[inline]
    pub(crate) fn id(&self, i: usize) -> usize {
        self.ids[i] as usize
    }

    /// String `id` of the string table.
    #[inline]
    pub(crate) fn string(&self, id: usize) -> &'a str {
        let start = self.offsets[id] as usize;
        let end = self.offsets[id + 1] as usize;
        // SAFETY: `data` must be valid UTF-8. All flamegraph inputs are UTF-8 encoded text.
        unsafe { std::str::from_utf8_unchecked(&self.data[start..end]) }
    }

    /// The number of strings in the string table.
    pub(crate) fn string_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// The id of `name` in the string table, if present.
    pub(crate) fn find(&self, name: &str) -> Option<usize> {
        (0..self.string_count()).find(|&id| self.string(id) == name)
    }
}

/// Get the children of node `i` as a slice of node IDs from the CSR children data.
#[inline]
pub(crate) fn get_children<'a>(children_offsets: &[i32], children_data: &'a [i32], i: usize) -> &'a [i32] {
    let start = children_offsets[i] as usize;
    let end = children_offsets[i + 1] as usize;
    &children_data[start..end]
}

/// Build a parent map from the CSR children data.
/// `parents[child] = parent_id`, or `-1` for the root.
pub(crate) fn build_parent_map(children_offsets: &[i32], children_data: &[i32]) -> Vec<i32> {
    let size = children_offsets.len().saturating_sub(1);
    let mut parents: Vec<i32> = vec![-1; size];
    for i in 0..size {
        for &child in get_children(children_offsets, children_data, i) {
            parents[child as usize] = i as i32;
        }
    }
    parents
}

/// A stack graph in flat Compressed Sparse Row (CSR) encoding for efficient transfer between Rust and JS.
///
/// Children are stored in CSR format:
///   children of node i = children_data[children_offsets[i] .. children_offsets[i+1]]
///   children are sorted alphabetically by node name.
///
/// Names are interned: each distinct name is stored once in a string table of UTF-8 byte buffers
/// with offset arrays, and each node refers to its name by id:
///   name of node i = string name_ids[i]
///   string s = names_data[names_offsets[s] .. names_offsets[s+1]]
/// Display names are indexed by the same ids:
///   display name of node i = display_names_data[display_names_offsets[name_ids[i]] ..]
#[wasm_bindgen]
#[derive(Clone)]
pub struct WasmStackGraph {
    pub(crate) children_offsets: Vec<i32>,
    pub(crate) children_data: Vec<i32>,
    pub(crate) name_ids: Vec<i32>,
    pub(crate) names_data: Vec<u8>,
    pub(crate) names_offsets: Vec<i32>,
    pub(crate) display_names_data: Vec<u8>,
    pub(crate) display_names_offsets: Vec<i32>,
    pub(crate) values: Vec<i64>,
}

#[wasm_bindgen]
impl WasmStackGraph {
    pub fn node_count(&self) -> u32 {
        self.name_ids.len() as u32
    }

    pub fn children_offsets(&self) -> Vec<i32> {
        self.children_offsets.clone()
    }

    pub fn children_data(&self) -> Vec<i32> {
        self.children_data.clone()
    }

    pub fn name_ids(&self) -> Vec<i32> {
        self.name_ids.clone()
    }

    pub fn names_data(&self) -> Vec<u8> {
        self.names_data.clone()
    }

    pub fn names_offsets(&self) -> Vec<i32> {
        self.names_offsets.clone()
    }

    pub fn display_names_data(&self) -> Vec<u8> {
        self.display_names_data.clone()
    }

    pub fn display_names_offsets(&self) -> Vec<i32> {
        self.display_names_offsets.clone()
    }

    pub fn values(&self) -> Vec<i64> {
        self.values.clone()
    }
}

/// The arrays of a [`WasmStackGraph`], owned and directly accessible, for native (non-WASM)
/// consumers such as the CLI. Same layout as [`WasmStackGraph`].
pub struct StackGraphData {
    pub children_offsets: Vec<i32>,
    pub children_data: Vec<i32>,
    pub name_ids: Vec<i32>,
    pub names_data: Vec<u8>,
    pub names_offsets: Vec<i32>,
    pub display_names_data: Vec<u8>,
    pub display_names_offsets: Vec<i32>,
    pub values: Vec<i64>,
}

impl WasmStackGraph {
    pub(crate) fn names(&self) -> Names<'_> {
        Names::new(&self.name_ids, &self.names_data, &self.names_offsets)
    }

    /// Moves the graph's arrays out without copying them.
    pub fn into_data(self) -> StackGraphData {
        StackGraphData {
            children_offsets: self.children_offsets,
            children_data: self.children_data,
            name_ids: self.name_ids,
            names_data: self.names_data,
            names_offsets: self.names_offsets,
            display_names_data: self.display_names_data,
            display_names_offsets: self.display_names_offsets,
            values: self.values,
        }
    }
}

/// The result of a diff computation: the merged graph, whose `values` array
/// holds graph A's per-node inclusive sample counts, with graph B's counts
/// alongside in `b_values`. All diff measures (deltas, widths, colors) are
/// derived from the two count arrays client-side.
#[wasm_bindgen]
pub struct WasmDiffGraph {
    pub(crate) graph: WasmStackGraph,
    pub(crate) b_values: Vec<i64>,
}

#[wasm_bindgen]
impl WasmDiffGraph {
    /// Sample counts from graph B, indexed by node ID.
    /// Graph A's counts are the inner graph's `values` array.
    pub fn b_values(&self) -> Vec<i64> {
        self.b_values.clone()
    }

    pub fn into_graph(self) -> WasmStackGraph {
        self.graph
    }
}
