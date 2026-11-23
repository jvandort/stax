use std::collections::HashMap;
use std::sync::OnceLock;

use flamegraph_wasm::{CallGraph, StackGraphData, WasmStackGraph};

/// How a diff graph turns each node's two sample counts into the single value that commands rank,
/// budget and collapse by, as in the flamegraph's diff views. Δ is B − A.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum DiffMode {
    /// |Δ|: everything that changed. Subtrees whose changes cancel out have no value.
    Delta,
    /// Δ where it is positive: what got slower.
    Slower,
    /// −Δ where it is positive: what got faster.
    Faster,
}

impl DiffMode {
    fn measure(self, a: i64, b: i64) -> i64 {
        match self {
            DiffMode::Delta => (b - a).abs(),
            DiffMode::Slower => (b - a).max(0),
            DiffMode::Faster => (a - b).max(0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DiffMode::Delta => "delta",
            DiffMode::Slower => "slower",
            DiffMode::Faster => "faster",
        }
    }
}

/// What a graph's root node stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    /// A placeholder above a loaded profile's stacks, not a frame.
    Placeholder,
    /// A frame: a merged function's.
    Frame,
    /// The node an icicle graph was made from, with callers below it rather than callees.
    Inverted,
}

/// Graph B's counts of a diff graph, whose own `values` are graph A's.
struct Diff {
    b_values: Vec<i64>,
    mode: DiffMode,
}

/// A stack graph decoded by the core crate, with typed accessors over its CSR arrays.
///
/// A diff graph is viewed as a single graph: its values are the diff mode's measure of each
/// node's two counts, so commands work on it unchanged, and only the figures they print (the
/// `*_tag` methods) show both counts.
pub struct Graph {
    data: StackGraphData,
    diff: Option<Diff>,
    root: Root,
    /// Built on first use by the function commands.
    call_graph: OnceLock<CallGraph>,
}

impl Graph {
    pub fn new(graph: WasmStackGraph) -> Self {
        Graph { data: graph.into_data(), diff: None, root: Root::Placeholder, call_graph: OnceLock::new() }
    }

    /// A diff graph: `graph` aligns both sides by call path and holds A's counts, `b_values` B's.
    pub fn new_diff(graph: WasmStackGraph, b_values: Vec<i64>, mode: DiffMode) -> Self {
        Graph { data: graph.into_data(), diff: Some(Diff { b_values, mode }), root: Root::Placeholder, call_graph: OnceLock::new() }
    }

    /// The graph with `source`'s function IDs, so that a function keeps its ID in graphs derived
    /// from another: its names take `source`'s IDs and display names, and names `source` lacks
    /// get IDs after them.
    pub fn with_function_ids_of(self, source: &Graph) -> Self {
        let src = &source.data;
        let mut ids: HashMap<&str, i32> = HashMap::with_capacity(source.string_count());
        for f in 0..source.string_count() {
            ids.entry(source.function_name(f)).or_insert(f as i32);
        }
        let (mut names, mut names_offsets) = (src.names_data.clone(), src.names_offsets.clone());
        let (mut display, mut display_offsets) = (src.display_names_data.clone(), src.display_names_offsets.clone());
        let mut new_id = Vec::with_capacity(self.string_count());
        for f in 0..self.string_count() {
            let name = self.function_name(f);
            let id = match ids.get(name) {
                Some(&id) => id,
                None => {
                    let id = names_offsets.len() as i32 - 1;
                    names.extend_from_slice(name.as_bytes());
                    names_offsets.push(names.len() as i32);
                    display.extend_from_slice(self.function_display_name(f).as_bytes());
                    display_offsets.push(display.len() as i32);
                    id
                }
            };
            new_id.push(id);
        }
        let name_ids = self.data.name_ids.iter().map(|&s| new_id[s as usize]).collect();
        let data = StackGraphData {
            name_ids,
            names_data: names,
            names_offsets,
            display_names_data: display,
            display_names_offsets: display_offsets,
            ..self.data
        };
        Graph { data, call_graph: OnceLock::new(), ..self }
    }

    /// The graph with its root standing for `root`.
    pub fn with_root(self, root: Root) -> Self {
        Graph { root, ..self }
    }

    pub fn root(&self) -> Root {
        self.root
    }

    pub fn is_diff(&self) -> bool {
        self.diff.is_some()
    }

    /// The underlying arrays, for passing to the core crate's graph transformations.
    pub fn data(&self) -> &StackGraphData {
        &self.data
    }

    pub fn node_count(&self) -> usize {
        self.data.name_ids.len()
    }

    pub fn children(&self, i: usize) -> impl ExactSizeIterator<Item = usize> + '_ {
        let start = self.data.children_offsets[i] as usize;
        let end = self.data.children_offsets[i + 1] as usize;
        self.data.children_data[start..end].iter().map(|&c| c as usize)
    }

    /// The graph's functions: one per distinct name, identified by its name id.
    pub fn call_graph(&self) -> &CallGraph {
        self.call_graph.get_or_init(|| {
            let d = &self.data;
            let root_is_frame = self.root == Root::Frame;
            CallGraph::new(&d.children_offsets, &d.children_data, &d.name_ids, self.string_count(), &d.values, root_is_frame)
        })
    }

    /// The number of distinct names, which is one more than the highest function id.
    pub fn string_count(&self) -> usize {
        self.data.names_offsets.len() - 1
    }

    /// The function of node `i`.
    pub fn function_of(&self, i: usize) -> usize {
        self.data.name_ids[i] as usize
    }

    /// The full name of function `f`.
    pub fn function_name(&self, f: usize) -> &str {
        decode(&self.data.names_data, &self.data.names_offsets, f)
    }

    /// The abbreviated name of function `f`, as shown in the flamegraph.
    pub fn function_display_name(&self, f: usize) -> &str {
        decode(&self.data.display_names_data, &self.data.display_names_offsets, f)
    }

    /// The full name of node `i`.
    pub fn name(&self, i: usize) -> &str {
        decode(&self.data.names_data, &self.data.names_offsets, self.data.name_ids[i] as usize)
    }

    /// The abbreviated name of node `i`, as shown in the flamegraph.
    pub fn display_name(&self, i: usize) -> &str {
        decode(&self.data.display_names_data, &self.data.display_names_offsets, self.data.name_ids[i] as usize)
    }

    /// Inclusive samples of node `i`; for a diff, the diff mode's measure of its two counts.
    pub fn value(&self, i: usize) -> i64 {
        match &self.diff {
            None => self.data.values[i],
            Some(diff) => diff.mode.measure(self.data.values[i], diff.b_values[i]),
        }
    }

    /// Samples of node `i` not attributed to any child; for a diff, the measure of its self counts.
    pub fn self_value(&self, i: usize) -> i64 {
        match &self.diff {
            None => self.value(i) - self.children(i).map(|c| self.value(c)).sum::<i64>(),
            Some(diff) => {
                let (a, b) = self.self_counts(i);
                diff.mode.measure(a, b)
            }
        }
    }

    /// A diff node's inclusive counts in A and B.
    fn counts(&self, diff: &Diff, i: usize) -> (i64, i64) {
        (self.data.values[i], diff.b_values[i])
    }

    /// A diff node's self counts in A and B.
    fn self_counts(&self, i: usize) -> (i64, i64) {
        let diff = self.diff.as_ref().expect("a diff graph");
        let (mut a, mut b) = self.counts(diff, i);
        for c in self.children(i) {
            let (ca, cb) = self.counts(diff, c);
            a -= ca;
            b -= cb;
        }
        (a, b)
    }

    /// The names of the columns `cells` fills: total and self samples, or for a diff, A's and B's
    /// totals, their Δ, and the Δ of self samples.
    pub fn columns(&self) -> &'static [&'static str] {
        match &self.diff {
            None => &["total", "self"],
            Some(_) => &["A", "B", "Δ", "self Δ"],
        }
    }

    /// Node `i`'s figures, one per column of `columns`.
    pub fn cells(&self, i: usize) -> Vec<String> {
        match &self.diff {
            None => vec![format_count(self.value(i)), format_count(self.self_value(i))],
            Some(diff) => {
                let (a, b) = self.counts(diff, i);
                let (self_a, self_b) = self.self_counts(i);
                vec![format_count(a), format_count(b), format_delta(b - a), format_delta(self_b - self_a)]
            }
        }
    }

    /// The graph's samples, as `list` and `load` print them: the total, or for a diff, A → B.
    pub fn samples_cell(&self) -> String {
        match &self.diff {
            None => format_count(self.total()),
            Some(diff) => {
                let (a, b) = self.counts(diff, 0);
                format!("{} → {}", format_count(a), format_count(b))
            }
        }
    }

    /// Samples in the whole graph.
    pub fn total(&self) -> i64 {
        self.value(0)
    }

    /// `result[i]` is the parent of node `i`, or `None` for the root.
    pub fn parent_map(&self) -> Vec<Option<usize>> {
        let mut parents = vec![None; self.node_count()];
        for i in 0..self.node_count() {
            for child in self.children(i) {
                parents[child] = Some(i);
            }
        }
        parents
    }
}

/// String `i` of a string table.
fn decode<'a>(data: &'a [u8], offsets: &[i32], i: usize) -> &'a str {
    let bytes = &data[offsets[i] as usize..offsets[i + 1] as usize];
    std::str::from_utf8(bytes).unwrap_or("<invalid UTF-8>")
}

/// Formats an integer with thousands separators, e.g. `1,234,567`.
pub fn format_count(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Formats a change with its sign, e.g. `+34`, `-3`, `0`.
pub fn format_delta(delta: i64) -> String {
    if delta > 0 { format!("+{}", format_count(delta)) } else { format_count(delta) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1,000");
        assert_eq!(format_count(1234567), "1,234,567");
        assert_eq!(format_count(-1234), "-1,234");
    }

    #[test]
    fn test_format_delta() {
        assert_eq!(format_delta(34), "+34");
        assert_eq!(format_delta(-1234), "-1,234");
        assert_eq!(format_delta(0), "0");
    }
}
