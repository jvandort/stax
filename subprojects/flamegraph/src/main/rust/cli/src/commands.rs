// Graph queries, ported from the MCP server's tools. Each returns the text to print on success,
// or an error message.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use flamegraph_wasm::{wasm_icicle_graph, wasm_merge_children, wasm_simplify_graph, JfrParser, StacksParser};
use regex::{Regex, RegexBuilder};

use crate::cli::Target;
use crate::graph::{format_count, format_samples, Graph};
use crate::summary::{build_tree, render_tree, WeightedTree};

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
            return Err("No profiles loaded.".to_string());
        }
        match self.graphs.get(&graph_id) {
            Some(entry) => Ok((entry.name.clone(), entry.graph.clone())),
            None => Err(format!("Graph {graph_id} not found. Available IDs: {}", self.ids())),
        }
    }

    fn ids(&self) -> String {
        self.graphs.keys().map(u32::to_string).collect::<Vec<_>>().join(", ")
    }

    fn summary_line(&self, id: u32) -> String {
        let GraphEntry { name, graph } = &self.graphs[&id];
        format!(
            "  [{id}] {name} — {} nodes, {} total samples, root node: 0",
            format_count(graph.node_count() as i64),
            format_count(graph.total())
        )
    }

    pub fn list(&self) -> Outcome {
        if self.graphs.is_empty() {
            return Ok("No profiles loaded.".to_string());
        }
        let lines: Vec<_> = self.graphs.keys().map(|&id| self.summary_line(id)).collect();
        Ok(format!("{} profile(s) loaded:\n{}", self.graphs.len(), lines.join("\n")))
    }

    /// Registers graphs parsed by [`parse_profile`] and describes them.
    pub fn load(&mut self, graphs: Vec<(String, Graph)>) -> Outcome {
        let ids: Vec<_> = graphs.into_iter().map(|(name, graph)| self.register(name, graph)).collect();
        let lines: Vec<_> = ids.iter().map(|&id| self.summary_line(id)).collect();
        Ok(format!("Loaded {} graph(s):\n{}", ids.len(), lines.join("\n")))
    }

    pub fn unload(&mut self, graph_id: u32) -> Outcome {
        match self.graphs.remove(&graph_id) {
            Some(GraphEntry { name, .. }) => Ok(format!(
                "Unloaded graph {graph_id} \"{name}\". {} graph(s) remaining.",
                self.graphs.len()
            )),
            None => Err(format!("Graph {graph_id} not found. Available IDs: {}", self.ids())),
        }
    }
}

/// Parses a profile into named graphs: one per sample kind for `.jfr`, otherwise one graph from
/// `-stacks.txt` text, gunzipped first for `.gz`.
pub fn parse_profile(path: &Path) -> Result<Vec<(String, Graph)>, String> {
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
        return Err(format!("Node ID {node_id} out of range (0–{}).", graph.node_count() - 1));
    }
    Ok(())
}

fn compile_pattern(pattern: &str) -> Result<Regex, String> {
    RegexBuilder::new(pattern).case_insensitive(true).build().map_err(|e| format!("Invalid regex: {e}"))
}

/// Resolves a command's target node: the given node ID, or the highest-sample node (excluding the
/// root) whose name matches the pattern.
fn resolve_target(graph: &Graph, name: &str, target: &Target) -> Result<usize, String> {
    if let Some(node_id) = target.node {
        check_node(graph, node_id)?;
        return Ok(node_id);
    }
    let pattern = target.pattern.as_deref().expect("clap requires --node or --pattern");
    let regex = compile_pattern(pattern)?;
    let mut best: Option<usize> = None;
    for i in 1..graph.node_count() {
        if regex.is_match(graph.name(i)) && best.is_none_or(|b| graph.value(i) > graph.value(b)) {
            best = Some(i);
        }
    }
    best.ok_or_else(|| format!("No nodes matching /{pattern}/i in \"{name}\"."))
}

pub fn search(name: &str, graph: &Graph, pattern: &str, limit: usize, offset: usize, root: usize) -> Outcome {
    check_node(graph, root)?;
    let regex = compile_pattern(pattern)?;
    let total = graph.total();
    let scope = if root != 0 { format!(" under [{root}]") } else { String::new() };

    // Collect nodes to search: if rooted at a subtree, BFS from the root.
    let search_nodes: Vec<usize> = if root == 0 {
        (1..graph.node_count()).collect()
    } else {
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let nid = queue[head];
            queue.extend(graph.children(nid));
            head += 1;
        }
        queue
    };

    let mut matches: Vec<usize> = search_nodes.into_iter().filter(|&i| regex.is_match(graph.name(i))).collect();
    if matches.is_empty() {
        return Ok(format!("No nodes matching /{pattern}/i in \"{name}\"{scope}."));
    }
    matches.sort_by(|&a, &b| graph.value(b).cmp(&graph.value(a)));
    let shown = &matches[offset.min(matches.len())..(offset + limit).min(matches.len())];
    let parents = graph.parent_map();

    // Combined total counts each stack once: a match nested under another match (recursion,
    // nested calls) is already covered by its ancestor's inclusive value, so only top-most matches
    // contribute. This keeps the percentage ≤100% and makes it read as "samples in stacks passing
    // through at least one match".
    let mut is_match = vec![false; graph.node_count()];
    for &m in &matches {
        is_match[m] = true;
    }
    let mut match_total = 0;
    for &m in &matches {
        let mut cur = parents[m];
        while let Some(c) = cur.filter(|&c| !is_match[c]) {
            cur = parents[c];
        }
        if cur.is_none() {
            match_total += graph.value(m);
        }
    }

    let lines: Vec<_> = shown
        .iter()
        .map(|&nid| {
            let parent = match parents[nid] {
                None => "root".to_string(),
                Some(p) => format!("[{p}] {}", graph.display_name(p)),
            };
            let self_value = graph.self_value(nid);
            let self_str =
                if self_value > 0 { format!(", self: {}", format_samples(self_value, total)) } else { String::new() };
            format!(
                "  [{nid}] {}, {}{self_str}, parent: {parent}",
                graph.display_name(nid),
                format_samples(graph.value(nid), total)
            )
        })
        .collect();
    let remaining = matches.len().saturating_sub(offset + shown.len());
    let suffix = if remaining > 0 { format!("\n  ({remaining} more not shown)") } else { String::new() };
    Ok(format!(
        "{} node(s) matching /{pattern}/i in \"{name}\"{scope} — combined: {}, nested matches counted once (showing {} from offset {offset}):\n{}{suffix}",
        matches.len(),
        format_samples(match_total, total),
        shown.len(),
        lines.join("\n")
    ))
}

pub fn node_details(name: &str, graph: &Graph, node_id: usize, parent_depth: i64) -> Outcome {
    check_node(graph, node_id)?;
    let total = graph.total();
    let parents = graph.parent_map();
    let mut chain = Vec::new();
    let mut cur = parents[node_id];
    while let Some(c) = cur {
        if parent_depth != -1 && chain.len() as i64 >= parent_depth {
            break;
        }
        chain.push(format!("  [{c}] {}", graph.display_name(c)));
        cur = parents[c];
    }
    let mut parent_chain = chain.join("\n");
    if cur.is_some() {
        parent_chain.push_str(if chain.is_empty() { "  (truncated)" } else { "\n  (truncated)" });
    } else if chain.is_empty() {
        parent_chain.push_str("  (root)");
    }
    Ok([
        format!("Node [{node_id}] in \"{name}\":"),
        format!("  Name: {}", graph.display_name(node_id)),
        format!("  Total samples: {}", format_samples(graph.value(node_id), total)),
        format!("  Self samples:  {}", format_samples(graph.self_value(node_id), total)),
        format!("  Children: {}", graph.children(node_id).len()),
        "  Parent chain (node → root):".to_string(),
        parent_chain,
    ]
    .join("\n"))
}

pub fn children(name: &str, graph: &Graph, node_id: usize, limit: i64) -> Outcome {
    check_node(graph, node_id)?;
    let parent_value = graph.value(node_id);
    let mut children: Vec<usize> = graph.children(node_id).collect();
    if children.is_empty() {
        return Ok(format!("Node [{node_id}] \"{}\" has no children.", graph.display_name(node_id)));
    }
    children.sort_by(|&a, &b| graph.value(b).cmp(&graph.value(a)));
    let shown = if limit == -1 { &children[..] } else { &children[..(limit as usize).min(children.len())] };
    let lines: Vec<_> = shown
        .iter()
        .map(|&c| {
            format!(
                "  [{c}] {}  total: {}  self: {}",
                graph.display_name(c),
                format_samples(graph.value(c), parent_value),
                format_samples(graph.self_value(c), parent_value)
            )
        })
        .collect();
    let suffix = if shown.len() < children.len() {
        format!("\n  ({} more not shown)", children.len() - shown.len())
    } else {
        String::new()
    };
    Ok(format!(
        "Children of [{node_id}] \"{}\" in \"{name}\" ({}/{}):\n{}{suffix}",
        graph.display_name(node_id),
        shown.len(),
        children.len(),
        lines.join("\n")
    ))
}

/// Merges every call site of the target's function into a new graph. Returns the new graph's
/// name, the graph, and a note to append to the creation message.
pub fn aggregate(name: &str, graph: &Graph, target: &Target) -> Result<(String, Graph, String), String> {
    let node_id = resolve_target(graph, name, target)?;
    let node_name = graph.name(node_id);
    let d = graph.data();
    let merged = wasm_merge_children(
        &d.children_offsets,
        &d.children_data,
        &d.names_data,
        &d.names_offsets,
        &d.values,
        node_name,
    )
    .map_err(|e| e.to_string())?;
    let merged = Graph::new(merged);

    // Sum inclusive values of all matching nodes in the source graph. If this exceeds the merged
    // root, the difference represents samples that passed through two or more matching nodes in
    // the same call stack (nested calls / recursion). Those samples are counted once per
    // occurrence in the sum but only once in the merged root, which is the authoritative figure.
    let source_sum: i64 = (0..graph.node_count()).filter(|&i| graph.name(i) == node_name).map(|i| graph.value(i)).sum();
    let merged_root = merged.total();
    let diff = source_sum - merged_root;
    let note = if diff > 0 {
        format!(
            " Note: individual node inclusive values sum to {} but merged root is {} — the {}-sample difference represents stacks where this function appears more than once (nested calls / recursion), counted once in the merged graph.",
            format_count(source_sum),
            format_count(merged_root),
            format_count(diff)
        )
    } else {
        String::new()
    };
    Ok((format!("{name} [merged: {}]", graph.display_name(node_id)), merged, note))
}

/// Builds an icicle graph of the subtree under the target. Returns the new graph's name and graph.
pub fn icicle(name: &str, graph: &Graph, target: &Target) -> Result<(String, Graph), String> {
    let node_id = resolve_target(graph, name, target)?;
    let d = graph.data();
    let icicle = wasm_icicle_graph(
        &d.children_offsets,
        &d.children_data,
        &d.names_data,
        &d.names_offsets,
        &d.values,
        node_id as u32,
        graph.node_count() as u32,
    );
    Ok((format!("{name} [icicle: {}]", graph.display_name(node_id)), Graph::new(icicle)))
}

/// Simplifies the subtree under a node. Returns the new graph's name and graph.
pub fn simplify(name: &str, graph: &Graph, node_id: usize) -> Result<(String, Graph), String> {
    check_node(graph, node_id)?;
    let d = graph.data();
    let simplified = wasm_simplify_graph(
        &d.children_offsets,
        &d.children_data,
        &d.names_data,
        &d.names_offsets,
        &d.values,
        node_id as u32,
    );
    let suffix = if node_id == 0 { String::new() } else { format!(": {}", graph.display_name(node_id)) };
    Ok((format!("{name} [simplified{suffix}]"), Graph::new(simplified)))
}

/// Describes a graph created by `aggregate` or `icicle`.
pub fn created(kind: &str, id: u32, name: &str, node_count: usize, note: &str) -> String {
    format!(
        "Created {kind} graph [{id}] \"{name}\" with {} nodes, root node: 0.{note}",
        format_count(node_count as i64)
    )
}

pub fn raw_name(graph: &Graph, node_id: usize) -> Outcome {
    check_node(graph, node_id)?;
    Ok(graph.name(node_id).to_string())
}

/// The child with the most samples; the first one on ties.
fn hottest_child(graph: &Graph, node_id: usize) -> Option<usize> {
    graph.children(node_id).reduce(|best, c| if graph.value(c) > graph.value(best) { c } else { best })
}

pub fn hot_path(name: &str, graph: &Graph, node_id: usize, depth_limit: Option<usize>, max_lines: usize, collapse: bool) -> Outcome {
    check_node(graph, node_id)?;
    let total = graph.total();
    let mut path = Vec::new();
    let mut cur = node_id;
    loop {
        path.push(cur);
        let Some(next) = hottest_child(graph, cur) else {
            break; // leaf
        };
        cur = next;
        if depth_limit.is_some_and(|limit| path.len() >= limit) {
            break; // depth limit
        }
    }

    // The depth limit was hit only if we stopped with children still below the path's end. Then
    // `cur` is the next unvisited node; count from there to the leaf.
    let mut depth_note = String::new();
    if let Some(limit) = depth_limit {
        if path.len() >= limit && graph.children(*path.last().unwrap()).len() > 0 {
            let mut true_depth = path.len() + 1;
            while let Some(next) = hottest_child(graph, cur) {
                cur = next;
                true_depth += 1;
            }
            depth_note = format!(" (truncated at depth {limit}; full hot path: {true_depth} nodes)");
        }
    }

    let line = |nid: usize| {
        format!(
            "  [{nid}] {}  total: {}  self: {}",
            graph.display_name(nid),
            format_samples(graph.value(nid), total),
            format_samples(graph.self_value(nid), total)
        )
    };

    // Collapse consecutive frames with the same inclusive value (pass-through wrappers). Stop
    // when we hit the line limit.
    let mut lines = Vec::new();
    let mut i = 0;
    while i < path.len() && lines.len() < max_lines {
        let value = graph.value(path[i]);
        lines.push(line(path[i]));
        let mut j = i + 1;
        if collapse {
            while j < path.len() && graph.value(path[j]) == value {
                j += 1;
            }
        }
        let skipped = j - i - 1;
        if skipped == 1 && lines.len() < max_lines {
            lines.push(line(path[i + 1]));
        } else if skipped > 1 && lines.len() + 1 < max_lines {
            lines.push(format!("  ... {skipped} frames with same total ..."));
            lines.push(line(path[j - 1]));
        }
        i = j;
    }
    let trunc_note = if i < path.len() { format!(" (truncated at {max_lines} lines)") } else { String::new() };
    Ok(format!(
        "Hot path from [{node_id}] in \"{name}\" ({} nodes, {} lines shown{depth_note}{trunc_note}):\n{}",
        path.len(),
        lines.len(),
        lines.join("\n")
    ))
}

/// The graph viewed top-down from a node.
struct ForwardView<'a>(&'a Graph);

impl WeightedTree for ForwardView<'_> {
    type Node = usize;

    fn display_name(&self, &n: &usize) -> String {
        self.0.display_name(n).to_string()
    }
    fn weight(&self, &n: &usize) -> i64 {
        self.0.value(n)
    }
    fn self_weight(&self, &n: &usize) -> i64 {
        self.0.self_value(n)
    }
    fn child_count(&self, &n: &usize) -> usize {
        self.0.children(n).len()
    }
    fn sorted_children(&self, &n: &usize) -> Vec<usize> {
        let mut children: Vec<usize> = self.0.children(n).collect();
        children.sort_by(|&a, &b| self.0.value(b).cmp(&self.0.value(a)));
        children
    }
    fn node_id(&self, &n: &usize) -> Option<usize> {
        Some(n)
    }
}

pub fn subtree(name: &str, graph: &Graph, node_id: usize, max_lines: usize, collapse: bool) -> Outcome {
    check_node(graph, node_id)?;
    let tree = build_tree(&ForwardView(graph), node_id, max_lines as i64, collapse);
    let mut lines = Vec::new();
    render_tree(&tree, graph.total(), "children", "", "", &mut lines);
    Ok(format!("Subtree summary of [{node_id}] in \"{name}\" ({} lines):\n{}", lines.len(), lines.join("\n")))
}

/// A caller set: real node IDs, all with the same name, each with the weight attributed to it
/// from below, in insertion order.
type CallerSet = Vec<(usize, i64)>;

/// The graph viewed bottom-up: a node's children are its callers, grouped by name.
struct CallerView<'a> {
    graph: &'a Graph,
    parents: Vec<Option<usize>>,
}

impl WeightedTree for CallerView<'_> {
    type Node = CallerSet;

    fn display_name(&self, cs: &CallerSet) -> String {
        self.graph.display_name(cs[0].0).to_string()
    }
    fn weight(&self, cs: &CallerSet) -> i64 {
        cs.iter().map(|&(_, w)| w).sum()
    }
    fn self_weight(&self, cs: &CallerSet) -> i64 {
        let parent_weight: i64 = cs.iter().filter(|&&(nid, _)| self.parents[nid].is_some()).map(|&(_, w)| w).sum();
        self.weight(cs) - parent_weight
    }
    fn child_count(&self, cs: &CallerSet) -> usize {
        let mut names: Vec<&str> = cs.iter().filter_map(|&(nid, _)| self.parents[nid]).map(|p| self.graph.name(p)).collect();
        names.sort_unstable();
        names.dedup();
        names.len()
    }
    fn sorted_children(&self, cs: &CallerSet) -> Vec<CallerSet> {
        let mut groups: Vec<CallerSet> = Vec::new();
        let mut group_of: HashMap<&str, usize> = HashMap::new();
        for &(nid, w) in cs {
            let Some(pid) = self.parents[nid] else { continue };
            let g = *group_of.entry(self.graph.name(pid)).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            match groups[g].iter_mut().find(|(id, _)| *id == pid) {
                Some((_, weight)) => *weight += w,
                None => groups[g].push((pid, w)),
            }
        }
        groups.sort_by_cached_key(|g| std::cmp::Reverse(self.weight(g)));
        groups
    }
}

pub fn callers(name: &str, graph: &Graph, target: &Target, max_lines: usize) -> Outcome {
    let node_id = resolve_target(graph, name, target)?;
    let target_name = graph.name(node_id);

    let parents = graph.parent_map();
    let is_instance: Vec<bool> = (0..graph.node_count()).map(|i| graph.name(i) == target_name).collect();
    let instance_count = is_instance.iter().filter(|&&b| b).count();

    // Start from the outermost instances only, each with its inclusive value. An instance nested
    // under another (recursion) is already covered by that ancestor's inclusive value, so counting
    // it too would count its samples once per level of recursion. This way every sample through
    // the function counts exactly once, as in `aggregate` and `search`.
    let outermost: CallerSet = (0..graph.node_count())
        .filter(|&i| is_instance[i])
        .filter(|&i| {
            let mut cur = parents[i];
            while let Some(c) = cur {
                if is_instance[c] {
                    return false;
                }
                cur = parents[c];
            }
            true
        })
        .map(|i| (i, graph.value(i)))
        .collect();
    let nested_note = if outermost.len() < instance_count { ", nested recursive calls counted once" } else { "" };

    let view = CallerView { graph, parents };
    let tree = build_tree(&view, outermost, max_lines as i64, false);
    let mut lines = Vec::new();
    render_tree(&tree, graph.total(), "callers", "", "", &mut lines);
    Ok(format!(
        "Callers of \"{}\" in \"{name}\" ({} instances{nested_note}, {} lines):\n{}",
        graph.display_name(node_id),
        format_count(instance_count as i64),
        lines.len(),
        lines.join("\n")
    ))
}
