// Graph queries. Each returns the text to print on success, or an error message.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use flamegraph_wasm::{
    wasm_diff_graphs, wasm_icicle_graph, wasm_merge_children, wasm_simplify_graph, CallGraph, JfrParser,
    StacksParser,
};
use regex::RegexBuilder;

use crate::cli::{NodeOrFunction, Page, RankBy};
use crate::graph::{format_count, DiffMode, Graph, Root};
use crate::summary::{build_tree, render_tree, SummaryNode, WeightedTree};
use crate::table::Table;

pub type Outcome = Result<String, String>;

pub struct GraphEntry {
    pub name: String,
    pub graph: Arc<Graph>,
}

/// The graphs loaded in the daemon. IDs are never reused, so an ID always refers to the graph it
/// was first given to, or to nothing.
#[derive(Default)]
pub struct Session {
    graphs: BTreeMap<u32, GraphEntry>,
    next_id: u32,
}

impl Session {
    pub fn register(&mut self, name: String, graph: Graph) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.graphs.insert(id, GraphEntry { name, graph: Arc::new(graph) });
        id
    }

    /// Looks up a graph, returning its name and a handle that stays valid after it is unloaded.
    pub fn get(&self, graph_id: u32) -> Result<(String, Arc<Graph>), String> {
        if self.graphs.is_empty() {
            return Err("No graphs loaded. Load a profile with `stax load <file>...`.".to_string());
        }
        match self.graphs.get(&graph_id) {
            Some(entry) => Ok((entry.name.clone(), entry.graph.clone())),
            None => Err(format!("Graph {graph_id} not found. Loaded: {}.", self.ids())),
        }
    }

    fn ids(&self) -> String {
        self.graphs.keys().map(u32::to_string).collect::<Vec<_>>().join(", ")
    }

    /// The graphs `ids` as a table of their sizes.
    fn table(&self, ids: impl IntoIterator<Item = u32>) -> String {
        let mut table = Table::new(["nodes", "samples"]);
        for id in ids {
            let GraphEntry { name, graph } = &self.graphs[&id];
            table.row(vec![format_count(graph.node_count() as i64), graph.samples_cell()], format!("[{id}] {name}"));
        }
        table.render(true).join("\n")
    }

    pub fn list(&self) -> Outcome {
        if self.graphs.is_empty() {
            return Ok("No graphs loaded.".to_string());
        }
        Ok(format!("{} loaded:\n{}", count_noun(self.graphs.len(), "graph"), self.table(self.graphs.keys().copied())))
    }

    /// Registers graphs parsed by [`parse_profile`] and describes them.
    pub fn load(&mut self, graphs: Vec<(String, Graph)>) -> Outcome {
        let ids: Vec<_> = graphs.into_iter().map(|(name, graph)| self.register(name, graph)).collect();
        Ok(format!("Loaded {}:\n{}", count_noun(ids.len(), "graph"), self.table(ids)))
    }

    pub fn unload(&mut self, graph_id: u32) -> Outcome {
        match self.graphs.remove(&graph_id) {
            Some(GraphEntry { name, .. }) => Ok(format!(
                "Unloaded graph {graph_id} \"{name}\". {} left.",
                count_noun(self.graphs.len(), "graph")
            )),
            None => Err(format!("Graph {graph_id} not found. Loaded: {}.", self.ids())),
        }
    }
}

/// `n` and the noun, plural unless n is 1: `1 graph`, `2 graphs`.
fn count_noun(n: usize, noun: &str) -> String {
    format!("{} {noun}{}", format_count(n as i64), if n == 1 { "" } else { "s" })
}

/// Parses a profile into named graphs: one per sample kind for `.jfr`, otherwise one graph from
/// collapsed stacks text, gunzipped first for `.gz`. Each is simplified unless `raw`, and fails if
/// it has no samples.
pub fn parse_profile(path: &Path, raw: bool) -> Result<Vec<(String, Graph)>, String> {
    let graphs = parse_raw_profile(path)?;
    graphs
        .into_iter()
        .map(|(name, graph)| {
            if graph.total() == 0 {
                return Err(format!("{}: no samples.", path.display()));
            }
            Ok(if raw { (format!("{name} [raw]"), graph) } else { (name, simplify(&graph)) })
        })
        .collect()
}

fn parse_raw_profile(path: &Path) -> Result<Vec<(String, Graph)>, String> {
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let file = File::open(path).map_err(|e| fail(&e))?;
    if name.ends_with(".jfr") {
        let mut parser = JfrParser::new();
        feed_all(file, |chunk| parser.feed(chunk)).map_err(|e| fail(&e))?;
        let graphs = parser.finish().map_err(|e| fail(&e))?.into_graphs();
        Ok(graphs.into_iter().map(|(kind, g)| (format!("{name} ({kind})"), Graph::new(g))).collect())
    } else {
        let mut parser = StacksParser::new("root");
        let reader: Box<dyn Read> =
            if name.ends_with(".gz") { Box::new(flate2::read::MultiGzDecoder::new(file)) } else { Box::new(file) };
        feed_all(reader, |chunk| parser.feed(chunk)).map_err(|e| fail(&e))?;
        Ok(vec![(name, Graph::new(parser.finish().map_err(|e| fail(&e))?))])
    }
}

/// Reads `reader` to the end in chunks, passing each to `feed`.
fn feed_all<E: std::fmt::Display>(
    mut reader: impl Read,
    mut feed: impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<(), String> {
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => feed(&buf[..n]).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn check_node(graph: &Graph, node_id: usize) -> Result<(), String> {
    if node_id >= graph.node_count() {
        return Err(format!("Node {node_id} out of range (0–{}).", graph.node_count() - 1));
    }
    Ok(())
}

/// Fails for diff graphs: commands that derive a new graph, or that add up samples across nodes,
/// only work on stack graphs.
fn check_not_diff(graph: &Graph, command: &str) -> Result<(), String> {
    if graph.is_diff() {
        return Err(format!("{command} is not supported for diff graphs."));
    }
    Ok(())
}

/// Fails for graphs the function commands don't support: diffs, and icicle graphs, whose callers
/// and callees are swapped.
fn check_functions_supported(graph: &Graph, command: &str) -> Result<(), String> {
    check_not_diff(graph, command)?;
    if graph.root() == Root::Inverted {
        return Err(format!("{command} is not supported for icicle graphs; use the graph it was made from."));
    }
    Ok(())
}

/// Fails unless `f` is a function of the graph: a name some frame node has.
fn check_function(name: &str, graph: &Graph, f: usize) -> Result<(), String> {
    if f >= graph.string_count() || graph.call_graph().instances(f).is_empty() {
        return Err(format!("No function f{f} in \"{name}\". Find function IDs with `search` or `top`."));
    }
    Ok(())
}

/// `[123] name`, or `[0] root` for a placeholder root.
fn node_label(graph: &Graph, n: usize) -> String {
    format!("[{n}] {}", graph.display_name(n))
}

/// `[f12] name`.
fn function_label(graph: &Graph, f: usize) -> String {
    format!("[f{f}] {}", graph.function_display_name(f))
}

/// ` under [123] name`, or nothing for the root.
fn under_text(graph: &Graph, under: usize) -> String {
    if under == 0 { String::new() } else { format!(" under {}", node_label(graph, under)) }
}

/// The footer of a list with `rest` entries not shown.
fn more_footer(rest: usize, extra: &str, options: &str) -> Option<String> {
    (rest > 0).then(|| format!("{} more{extra}. Show more with {options}.", format_count(rest as i64)))
}

/// The entries of `items` a page shows.
fn page_of<T>(items: &[T], page: Page) -> &[T] {
    let start = page.offset.min(items.len());
    let end = if page.limit == -1 { items.len() } else { (start + page.limit as usize).min(items.len()) };
    &items[start..end]
}

/// The nodes of the subtree under `root`, in breadth-first order, `root` first.
fn subtree_nodes(graph: &Graph, root: usize) -> Vec<usize> {
    let mut order = vec![root];
    let mut head = 0;
    while head < order.len() {
        let n = order[head];
        order.extend(graph.children(n));
        head += 1;
    }
    order
}

/// The subtree under a node as stack graph arrays, with the graph's string table: `order[i]` is
/// the graph's node that is node `i` here, and `under` is node 0.
struct SubtreeArrays {
    order: Vec<usize>,
    children_offsets: Vec<i32>,
    children_data: Vec<i32>,
    name_ids: Vec<i32>,
    values: Vec<i64>,
}

impl SubtreeArrays {
    fn new(graph: &Graph, under: usize) -> Self {
        let order = subtree_nodes(graph, under);
        let mut new_id = vec![0i32; graph.node_count()];
        for (i, &n) in order.iter().enumerate() {
            new_id[n] = i as i32;
        }
        let mut children_offsets = Vec::with_capacity(order.len() + 1);
        let mut children_data = Vec::with_capacity(order.len());
        children_offsets.push(0);
        for &n in &order {
            children_data.extend(graph.children(n).map(|c| new_id[c]));
            children_offsets.push(children_data.len() as i32);
        }
        let name_ids = order.iter().map(|&n| graph.function_of(n) as i32).collect();
        let values = order.iter().map(|&n| graph.value(n)).collect();
        SubtreeArrays { order, children_offsets, children_data, name_ids, values }
    }
}

/// The call graph of the samples under node `under`, with the graph's function IDs: the graph's
/// own for the root, else one built from the subtree, whose root is a frame.
fn scoped_call_graph(graph: &Graph, under: usize) -> ScopedCallGraph<'_> {
    if under == 0 {
        return ScopedCallGraph::Whole(graph.call_graph());
    }
    let sub = SubtreeArrays::new(graph, under);
    let call_graph = CallGraph::new(
        &sub.children_offsets,
        &sub.children_data,
        &sub.name_ids,
        graph.string_count(),
        &sub.values,
        true,
    );
    ScopedCallGraph::Subtree(call_graph, sub.order)
}

enum ScopedCallGraph<'a> {
    Whole(&'a CallGraph),
    /// A subtree's call graph, and the graph's node for each of its nodes.
    Subtree(CallGraph, Vec<usize>),
}

impl ScopedCallGraph<'_> {
    /// The graph's node ID of the call graph's node `n`.
    fn node(&self, n: u32) -> usize {
        match self {
            ScopedCallGraph::Whole(_) => n as usize,
            ScopedCallGraph::Subtree(_, order) => order[n as usize],
        }
    }
}

impl std::ops::Deref for ScopedCallGraph<'_> {
    type Target = CallGraph;
    fn deref(&self) -> &CallGraph {
        match self {
            ScopedCallGraph::Whole(g) => g,
            ScopedCallGraph::Subtree(g, _) => g,
        }
    }
}

/// Fails unless function `f` has samples under node `under`.
fn check_function_under(name: &str, graph: &Graph, f: usize, under: usize, call_graph: &CallGraph) -> Result<(), String> {
    if call_graph.instances(f).is_empty() {
        return Err(format!("{} has no samples in \"{name}\"{}.", function_label(graph, f), under_text(graph, under)));
    }
    Ok(())
}

/// `top` and `search`: functions ranked by `by`, those whose names match `pattern` if given,
/// counting only the samples under node `under`.
pub fn rank_functions(
    command: &str,
    name: &str,
    graph: &Graph,
    pattern: Option<&str>,
    by: RankBy,
    under: usize,
    page: Page,
) -> Outcome {
    check_functions_supported(graph, command)?;
    check_node(graph, under)?;
    let regex = pattern
        .map(|p| RegexBuilder::new(p).case_insensitive(true).build().map_err(|e| format!("Invalid pattern: {e}")))
        .transpose()?;
    let call_graph = scoped_call_graph(graph, under);
    let rank = |f: usize| match by {
        RankBy::SelfSamples => call_graph.self_value(f),
        RankBy::Total => call_graph.inclusive(f),
    };
    let all = (0..graph.string_count()).filter(|&f| !call_graph.instances(f).is_empty()).count();
    let mut ranked: Vec<usize> = (0..graph.string_count())
        .filter(|&f| !call_graph.instances(f).is_empty())
        .filter(|&f| regex.as_ref().is_none_or(|r| r.is_match(graph.function_name(f))))
        .collect();
    ranked.sort_by(|&a, &b| {
        rank(b).cmp(&rank(a)).then_with(|| graph.function_display_name(a).cmp(graph.function_display_name(b)))
    });

    let (by_name, columns) = match by {
        RankBy::SelfSamples => ("self", ["self", "total"]),
        RankBy::Total => ("total", ["total", "self"]),
    };
    let scope = under_text(graph, under);
    let title = match pattern {
        None => format!("Functions in \"{name}\"{scope} by {by_name} samples ({}):", format_count(all as i64)),
        Some(p) => format!(
            "Functions matching /{p}/i in \"{name}\"{scope} by {by_name} samples ({} of {}):",
            format_count(ranked.len() as i64),
            format_count(all as i64)
        ),
    };
    if ranked.is_empty() {
        return Ok(format!("{title} none."));
    }
    let shown = page_of(&ranked, page);
    let mut table = Table::new(columns);
    for &f in shown {
        let (s, t) = (format_count(call_graph.self_value(f)), format_count(call_graph.inclusive(f)));
        let cells = match by {
            RankBy::SelfSamples => vec![s, t],
            RankBy::Total => vec![t, s],
        };
        table.row(cells, function_label(graph, f));
    }
    let rest = &ranked[(page.offset + shown.len()).min(ranked.len())..];
    // Self samples don't overlap between functions, so the rest's add up; totals do overlap.
    let extra = match by {
        RankBy::SelfSamples => {
            format!(" ({} self samples)", format_count(rest.iter().map(|&f| call_graph.self_value(f)).sum()))
        }
        RankBy::Total => String::new(),
    };
    if let Some(footer) = more_footer(rest.len(), &extra, "--limit or --offset") {
        table.text(footer);
    }
    Ok(format!("{title}\n{}", table.render(true).join("\n")))
}

pub fn node_details(name: &str, graph: &Graph, node_id: usize, parent_depth: i64) -> Outcome {
    check_node(graph, node_id)?;
    let parents = graph.parent_map();
    let mut table = Table::new(graph.columns().iter().copied());
    table.row(graph.cells(node_id), node_label(graph, node_id));
    let mut chain = Vec::new();
    let mut cur = parents[node_id];
    while let Some(c) = cur {
        chain.push(c);
        cur = parents[c];
    }
    if !chain.is_empty() {
        table.text("Parents, nearest first:");
        let shown = if parent_depth == -1 { chain.len() } else { chain.len().min(parent_depth as usize) };
        for &p in &chain[..shown] {
            table.row(graph.cells(p), node_label(graph, p));
        }
        if shown < chain.len() {
            table.text(format!("{} more. Show all with --parent-depth -1.", chain.len() - shown));
        }
    }
    let function = if node_id == 0 && graph.root() != Root::Frame {
        String::new()
    } else {
        format!(", function [f{}]", graph.function_of(node_id))
    };
    Ok(format!("Node [{node_id}] in \"{name}\"{function}:\n{}", table.render(true).join("\n")))
}

pub fn children(name: &str, graph: &Graph, node_id: usize, page: Page) -> Outcome {
    check_node(graph, node_id)?;
    let mut children: Vec<usize> = graph.children(node_id).collect();
    let title = format!("Children of {} in \"{name}\" ({}):", node_label(graph, node_id), children.len());
    if children.is_empty() {
        return Ok(format!("{title} none."));
    }
    children.sort_by(|&a, &b| graph.value(b).cmp(&graph.value(a)).then(a.cmp(&b)));
    let shown = page_of(&children, page);
    let mut table = Table::new(graph.columns().iter().copied());
    for &c in shown {
        table.row(graph.cells(c), node_label(graph, c));
    }
    let rest = children.len() - (page.offset.min(children.len()) + shown.len());
    if let Some(footer) = more_footer(rest, "", "--limit or --offset") {
        table.text(footer);
    }
    Ok(format!("{title}\n{}", table.render(true).join("\n")))
}

/// Merges every call site of function `f` into a new graph. Returns the new graph's name, the
/// graph, and a note to append to the creation message.
pub fn aggregate(name: &str, graph: &Graph, f: usize) -> Result<(String, Graph, String), String> {
    check_functions_supported(graph, "aggregate")?;
    check_function(name, graph, f)?;
    let merged = merge(graph, f, 0)?;

    // The function's nodes' totals add up to more than the merged root when some are inside
    // others (recursion): the merged root counts each sample once.
    let call_graph = graph.call_graph();
    let node_sum: i64 = call_graph.instances(f).iter().map(|&n| graph.value(n as usize)).sum();
    let nested = node_sum - merged.total();
    let note = if nested > 0 {
        format!(
            " Its root's {} samples count each sample once: {} more are recursive calls inside its other calls.",
            format_count(merged.total()),
            format_count(nested)
        )
    } else {
        String::new()
    };
    Ok((format!("{name} [merged: {}]", graph.function_display_name(f)), merged, note))
}

/// Function `f`'s call sites under node `under` merged into one graph rooted at it.
fn merge(graph: &Graph, f: usize, under: usize) -> Result<Graph, String> {
    let d = graph.data();
    let merged = if under == 0 {
        wasm_merge_children(
            &d.children_offsets,
            &d.children_data,
            &d.name_ids,
            &d.names_data,
            &d.names_offsets,
            &d.values,
            graph.function_name(f),
        )
    } else {
        let sub = SubtreeArrays::new(graph, under);
        wasm_merge_children(
            &sub.children_offsets,
            &sub.children_data,
            &sub.name_ids,
            &d.names_data,
            &d.names_offsets,
            &sub.values,
            graph.function_name(f),
        )
    };
    Ok(Graph::new(merged.map_err(|e| e.to_string())?).with_root(Root::Frame).with_function_ids_of(graph))
}

/// Builds an icicle graph of the subtree under a node. Returns the new graph's name and graph.
pub fn icicle(name: &str, graph: &Graph, node_id: usize) -> Result<(String, Graph), String> {
    check_not_diff(graph, "icicle")?;
    check_node(graph, node_id)?;
    let d = graph.data();
    let icicle = wasm_icicle_graph(
        &d.children_offsets,
        &d.children_data,
        &d.name_ids,
        &d.names_data,
        &d.names_offsets,
        &d.values,
        node_id as u32,
        graph.node_count() as u32,
    );
    Ok((format!("{name} [icicle: {}]", graph.display_name(node_id)), Graph::new(icicle).with_root(Root::Inverted).with_function_ids_of(graph)))
}

/// Merges the frames of each function that the profile tells apart by runtime details.
fn simplify(graph: &Graph) -> Graph {
    let d = graph.data();
    let simplified = wasm_simplify_graph(
        &d.children_offsets,
        &d.children_data,
        &d.name_ids,
        &d.names_data,
        &d.names_offsets,
        &d.values,
        0,
    );
    Graph::new(simplified).with_root(graph.root())
}

/// Compares two graphs: both are simplified and aligned by call path into one diff graph, valued
/// by `mode`. Returns the new graph's name and graph.
pub fn diff(a_name: &str, a: &Graph, b_name: &str, b: &Graph, mode: DiffMode) -> Result<(String, Graph), String> {
    check_not_diff(a, "diff")?;
    check_not_diff(b, "diff")?;
    let a_graph = a;
    let (a, b) = (a.data(), b.data());
    let diff = wasm_diff_graphs(
        &a.children_offsets,
        &a.children_data,
        &a.name_ids,
        &a.names_data,
        &a.names_offsets,
        &a.values,
        &b.children_offsets,
        &b.children_data,
        &b.name_ids,
        &b.names_data,
        &b.names_offsets,
        &b.values,
    );
    let b_values = diff.b_values();
    let graph = Graph::new_diff(diff.into_graph(), b_values, mode).with_function_ids_of(a_graph);
    Ok((format!("{a_name} vs {b_name} [diff: {}]", mode.name()), graph))
}

/// Describes a graph created by `aggregate`, `icicle` or `diff`.
pub fn created(id: u32, name: &str, node_count: usize, note: &str) -> String {
    format!("Created graph [{id}] \"{name}\" ({} nodes).{note}", format_count(node_count as i64))
}

pub fn raw_name(name: &str, graph: &Graph, id: NodeOrFunction) -> Outcome {
    match id {
        NodeOrFunction::Node(n) => {
            check_node(graph, n)?;
            Ok(graph.name(n).to_string())
        }
        NodeOrFunction::Function(f) => {
            if f >= graph.string_count() {
                return Err(format!("No function f{f} in \"{name}\"."));
            }
            Ok(graph.function_name(f).to_string())
        }
    }
}

/// The function card for `fn`, counting only the samples under node `under`.
pub fn function(name: &str, graph: &Graph, f: usize, under: usize, limit: i64) -> Outcome {
    check_functions_supported(graph, "fn")?;
    check_function(name, graph, f)?;
    check_node(graph, under)?;
    let call_graph = scoped_call_graph(graph, under);
    check_function_under(name, graph, f, under, &call_graph)?;
    let take = |n: usize| if limit == -1 { n } else { n.min(limit as usize) };
    let mut table = Table::new(["samples"]);

    // Edges, heaviest first, then by name.
    let mut edge_list = |title: &str, edges: &[(u32, i64)], stack_roots: Option<i64>| {
        let mut edges: Vec<(Option<usize>, i64)> = edges.iter().map(|&(g, w)| (Some(g as usize), w)).collect();
        // Stack roots: the samples where f is outermost, with no caller.
        if let Some(w) = stack_roots.filter(|&w| w > 0) {
            edges.push((None, w));
        }
        let stack_root = if under == 0 { "(stack root)".to_string() } else { format!("(caller above [{under}])") };
        let label = |g: Option<usize>| g.map_or(stack_root.clone(), |g| function_label(graph, g));
        edges.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| label(a.0).cmp(&label(b.0))));
        if edges.is_empty() {
            table.text(format!("{title}: none"));
            return;
        }
        table.text(format!("{title}:"));
        let shown = take(edges.len());
        for &(g, w) in &edges[..shown] {
            table.row(vec![format_count(w)], label(g));
        }
        let rest: i64 = edges[shown..].iter().map(|&(_, w)| w).sum();
        if let Some(footer) = more_footer(edges.len() - shown, &format!(" ({} samples)", format_count(rest)), "--limit")
        {
            table.text(footer);
        }
    };

    // Samples where f is the scope's outermost frame: the scope's root when it is a frame, else
    // the placeholder root's children.
    let root_samples: i64 = match graph.root() {
        _ if under != 0 => if graph.function_of(under) == f { graph.value(under) } else { 0 },
        Root::Frame => if graph.function_of(0) == f { graph.value(0) } else { 0 },
        _ => graph.children(0).filter(|&c| graph.function_of(c) == f).map(|c| graph.value(c)).sum(),
    };
    edge_list("Callers, with the samples on each call", call_graph.callers(f), Some(root_samples));
    edge_list("Callees", call_graph.callees(f), None);

    let mut nodes: Vec<usize> = call_graph.instances(f).iter().map(|&n| call_graph.node(n)).collect();
    nodes.sort_by(|&a, &b| graph.value(b).cmp(&graph.value(a)).then(a.cmp(&b)));
    let shown = take(nodes.len());
    table.text("Heaviest nodes:");
    for &n in &nodes[..shown] {
        table.row(vec![format_count(graph.value(n))], format!("[{n}]"));
    }
    if let Some(footer) = more_footer(nodes.len() - shown, "", "--limit") {
        table.text(footer);
    }
    Ok(format!(
        "{} in \"{name}\"{}: total {}, self {}, {}\n{}",
        function_label(graph, f),
        under_text(graph, under),
        format_count(call_graph.inclusive(f)),
        format_count(call_graph.self_value(f)),
        count_noun(nodes.len(), "node"),
        table.render(false).join("\n")
    ))
}

/// The graph viewed top-down from a node.
struct ForwardView<'a>(&'a Graph);

impl WeightedTree for ForwardView<'_> {
    type Node = usize;

    fn label(&self, &n: &usize) -> String {
        node_label(self.0, n)
    }
    fn cells(&self, &n: &usize) -> Vec<String> {
        self.0.cells(n)
    }
    fn weight(&self, &n: &usize) -> i64 {
        self.0.value(n)
    }
    fn child_count(&self, &n: &usize) -> usize {
        self.0.children(n).len()
    }
    fn sorted_children(&self, &n: &usize) -> Vec<usize> {
        let mut children: Vec<usize> = self.0.children(n).collect();
        children.sort_by(|&a, &b| self.0.value(b).cmp(&self.0.value(a)));
        children
    }
}

/// A tree view rendered under a title, with its column names, and a footer if the line budget cut
/// any line's `children` (the word for them: children, callers or callees).
fn tree_output(title: String, columns: &[&str], tree: &SummaryNode, children: &str) -> String {
    let mut table = Table::new(columns.iter().copied());
    render_tree(tree, &mut table);
    let cut = tree.cut_lines();
    if cut > 0 {
        let lines = if cut == 1 { "1 line has" } else { &format!("{} lines have", format_count(cut as i64)) };
        table.text(format!("{lines} {children} not shown. Show more with --max-lines <n>."));
    }
    format!("{title}\n{}", table.render(true).join("\n"))
}

pub fn subtree(name: &str, graph: &Graph, node_id: usize, max_lines: usize, collapse: bool) -> Outcome {
    check_node(graph, node_id)?;
    let tree = build_tree(&ForwardView(graph), node_id, max_lines as i64, collapse);
    Ok(tree_output(format!("Subtree of {} in \"{name}\":", node_label(graph, node_id)), graph.columns(), &tree, "children"))
}

/// A caller set: real node IDs, all with the same name, each with the weight attributed to it
/// from below, in insertion order.
type CallerSet = Vec<(usize, i64)>;

/// The graph viewed bottom-up: a node's children are its callers, grouped by name.
struct CallerView<'a> {
    graph: &'a Graph,
    /// Each node's parent, with chains cut where they leave the scope.
    parents: Vec<Option<usize>>,
}

impl WeightedTree for CallerView<'_> {
    type Node = CallerSet;

    fn label(&self, cs: &CallerSet) -> String {
        let n = cs[0].0;
        if n == 0 && self.graph.root() != Root::Frame {
            node_label(self.graph, 0)
        } else {
            function_label(self.graph, self.graph.function_of(n))
        }
    }
    fn cells(&self, cs: &CallerSet) -> Vec<String> {
        vec![format_count(self.weight(cs))]
    }
    fn weight(&self, cs: &CallerSet) -> i64 {
        cs.iter().map(|&(_, w)| w).sum()
    }
    fn child_count(&self, cs: &CallerSet) -> usize {
        let mut functions: Vec<usize> =
            cs.iter().filter_map(|&(nid, _)| self.parents[nid]).map(|p| self.graph.function_of(p)).collect();
        functions.sort_unstable();
        functions.dedup();
        functions.len()
    }
    fn sorted_children(&self, cs: &CallerSet) -> Vec<CallerSet> {
        let mut groups: Vec<CallerSet> = Vec::new();
        let mut group_of: HashMap<usize, usize> = HashMap::new();
        for &(nid, w) in cs {
            let Some(pid) = self.parents[nid] else { continue };
            let g = *group_of.entry(self.graph.function_of(pid)).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            match groups[g].iter_mut().find(|(id, _)| *id == pid) {
                Some((_, weight)) => *weight += w,
                None => groups[g].push((pid, w)),
            }
        }
        groups.sort_by_cached_key(|g| Reverse(self.weight(g)));
        groups
    }
}

pub fn callers(name: &str, graph: &Graph, f: usize, under: usize, max_lines: usize, collapse: bool) -> Outcome {
    check_functions_supported(graph, "callers")?;
    check_function(name, graph, f)?;
    check_node(graph, under)?;

    // Chains stop at `under`: its parent is out of scope.
    let mut parents = graph.parent_map();
    parents[under] = None;
    let mut in_scope = vec![under == 0; graph.node_count()];
    if under != 0 {
        for n in subtree_nodes(graph, under) {
            in_scope[n] = true;
        }
    }
    let is_instance = |n: usize| in_scope[n] && graph.function_of(n) == f;
    let instances: Vec<usize> =
        graph.call_graph().instances(f).iter().map(|&n| n as usize).filter(|&n| in_scope[n]).collect();
    if instances.is_empty() {
        return Ok(format!("No calls to {} in \"{name}\"{}.", function_label(graph, f), under_text(graph, under)));
    }

    // Start from the outermost instances only, each with its total. An instance nested under
    // another (recursion) is already covered by that ancestor's total, so counting it too would
    // count its samples once per level of recursion. This way every sample through the function
    // counts exactly once, as in `aggregate` and `top`.
    let outermost: CallerSet = instances
        .iter()
        .copied()
        .filter(|&i| {
            let mut cur = parents[i];
            while let Some(c) = cur {
                if is_instance(c) {
                    return false;
                }
                cur = parents[c];
            }
            true
        })
        .map(|i| (i, graph.value(i)))
        .collect();
    let recursion = if outermost.len() < instances.len() { " Recursive calls count once." } else { "" };

    let view = CallerView { graph, parents };
    let tree = build_tree(&view, outermost, max_lines as i64, collapse);
    let title = format!(
        "Callers of {} in \"{name}\"{}, callers below callees. A line's total is the samples of its call chain to the function.{recursion}",
        function_label(graph, f),
        under_text(graph, under)
    );
    Ok(tree_output(title, &["total"], &tree, "callers"))
}

/// A line of a call tree: function `f` (the scoped call graph's id) as called from `caller`, or the
/// root when `caller` is None, with the samples on that call.
#[derive(Clone, Copy)]
struct Call {
    caller: Option<u32>,
    f: u32,
    weight: i64,
}

/// A call graph drawn as a tree: each function's callees are shown once, under the call it is
/// `expanded_under`, and its other calls are leaves marked (*).
struct CallTreeView<'a> {
    calls: &'a Graph,
    /// The function each function is expanded under; None for the root and the unreached.
    expanded_under: Vec<Option<u32>>,
}

impl CallTreeView<'_> {
    fn is_expanded(&self, call: &Call) -> bool {
        call.caller == self.expanded_under[call.f as usize]
    }
}

impl WeightedTree for CallTreeView<'_> {
    type Node = Call;

    fn label(&self, call: &Call) -> String {
        let seen = if self.is_expanded(call) { "" } else { " (*)" };
        format!("[f{}] {}{seen}", call.f, self.calls.function_display_name(call.f as usize))
    }
    /// A (*) line's self samples are on the line where it is expanded, so its cell is blank.
    fn cells(&self, call: &Call) -> Vec<String> {
        let self_cell = if self.is_expanded(call) {
            format_count(self.calls.call_graph().self_value(call.f as usize))
        } else {
            String::new()
        };
        vec![format_count(call.weight), self_cell]
    }
    fn weight(&self, call: &Call) -> i64 {
        call.weight
    }
    /// A function called from several places: its self samples and callees are those of all its
    /// calls, so its total for all of them is shown too.
    fn note(&self, call: &Call) -> Option<String> {
        let inclusive = self.calls.call_graph().inclusive(call.f as usize);
        (self.is_expanded(call) && inclusive != call.weight)
            .then(|| format!("(in all calls: {})", format_count(inclusive)))
    }
    fn child_count(&self, call: &Call) -> usize {
        if self.is_expanded(call) { self.calls.call_graph().callees(call.f as usize).len() } else { 0 }
    }
    fn sorted_children(&self, call: &Call) -> Vec<Call> {
        if !self.is_expanded(call) {
            return Vec::new();
        }
        let mut children: Vec<Call> = self.calls.call_graph().callees(call.f as usize).iter()
            .map(|&(g, weight)| Call { caller: Some(call.f), f: g, weight })
            .collect();
        let name = |c: &Call| self.calls.function_display_name(c.f as usize);
        children.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| name(a).cmp(name(b))));
        children
    }
}

/// Draws the call graph under function `f`, merged across its call sites under node `under`, as a
/// tree.
pub fn calls(name: &str, graph: &Graph, f: usize, under: usize, max_lines: usize, collapse: bool) -> Outcome {
    check_functions_supported(graph, "calls")?;
    check_function(name, graph, f)?;
    check_node(graph, under)?;
    if under != 0 && !subtree_nodes(graph, under).into_iter().any(|n| graph.function_of(n) == f) {
        return Err(format!("{} has no samples in \"{name}\"{}.", function_label(graph, f), under_text(graph, under)));
    }
    // Scope to f's samples: merge its call sites, so that a callee's own callees count only the
    // calls made under f, not those from the rest of the profile.
    let calls = merge(graph, f, under)?;
    let call_graph = calls.call_graph();
    let root = calls.function_of(0) as u32;

    // Expand each function under its heaviest call from a function already expanded, heaviest
    // first (a maximum spanning tree grown from the root), so the samples that matter most are
    // the ones drawn in full, however deep.
    let mut expanded_under: Vec<Option<u32>> = vec![None; calls.string_count()];
    let mut reached = vec![false; calls.string_count()];
    reached[root as usize] = true;
    let mut frontier: BinaryHeap<(i64, Reverse<(u32, u32)>)> = call_graph
        .callees(root as usize)
        .iter()
        .map(|&(g, w)| (w, Reverse((root, g))))
        .collect();
    while let Some((_, Reverse((caller, g)))) = frontier.pop() {
        if std::mem::replace(&mut reached[g as usize], true) {
            continue;
        }
        expanded_under[g as usize] = Some(caller);
        frontier.extend(call_graph.callees(g as usize).iter().map(|&(h, w)| (w, Reverse((g, h)))));
    }

    let view = CallTreeView { calls: &calls, expanded_under };
    let tree = build_tree(&view, Call { caller: None, f: root, weight: calls.total() }, max_lines as i64, collapse);
    let title = format!(
        "Calls under {} in \"{name}\"{}, merged across its call sites. (*): its callees are shown at another line.",
        function_label(graph, f),
        under_text(graph, under)
    );
    Ok(tree_output(title, &["total", "self"], &tree, "callees"))
}
