use ahash::AHashMap;

use crate::get_children;

/// A call graph derived exactly from a stack graph: one function per distinct name, identified by
/// its name id, with weighted caller → callee edges.
///
/// Every figure is computed from the stack graph's full stacks, not estimated, and counts each
/// sample once, however often a function or edge repeats in its stack (recursion):
/// - a function's self value sums its nodes' self values;
/// - its inclusive value sums the values of its outermost nodes, i.e. those with no ancestor of
///   the same function (as `search`'s combined total and `aggregate`'s merged root count);
/// - an edge f → g sums the values of g-nodes whose parent is an f-node, counting a node only
///   when no ancestor pair is also f → g.
///
/// With recursion through several functions (f → g → f), a function's callers can add up to more
/// than its inclusive value: each edge is on the stack for those samples.
///
/// A loaded profile's root node is a placeholder, not a frame: unless `root_is_frame`, it is not a
/// function, has no self value, and edges from it are not recorded. A merged graph's root is the
/// merged function's frame, so it is one.
pub struct CallGraph {
    self_values: Vec<i64>,
    inclusive: Vec<i64>,
    /// Nodes of function f: `instance_data[instance_offsets[f]..instance_offsets[f + 1]]`.
    instance_offsets: Vec<u32>,
    instance_data: Vec<u32>,
    /// Edges sorted by caller then callee, with `callee_offsets[f]` the first of f's.
    callee_offsets: Vec<u32>,
    callees: Vec<(u32, i64)>,
    /// The same edges sorted by callee then caller, with `caller_offsets[g]` the first of g's.
    caller_offsets: Vec<u32>,
    callers: Vec<(u32, i64)>,
}

impl CallGraph {
    /// Builds the call graph of a stack graph with `string_count` names in its string table.
    pub fn new(
        children_offsets: &[i32],
        children_data: &[i32],
        name_ids: &[i32],
        string_count: usize,
        values: &[i64],
        root_is_frame: bool,
    ) -> Self {
        let node_count = name_ids.len();
        let mut self_values = vec![0i64; string_count];
        let mut inclusive = vec![0i64; string_count];
        // How many nodes of each function, and of each edge, are on the current path.
        let mut function_depth = vec![0u32; string_count];
        let mut edge_depth: AHashMap<(u32, u32), u32> = AHashMap::new();
        let mut edge_weights: AHashMap<(u32, u32), i64> = AHashMap::new();

        // Depth-first from the root, or from its children when it isn't a frame, with each node
        // visited on entry and again on exit (`exiting`) to pop it from the path. A node's parent
        // is usize::MAX when it has none that is a frame.
        let mut parents = vec![usize::MAX; node_count];
        let mut stack: Vec<(usize, bool)> = if root_is_frame {
            vec![(0, false)]
        } else {
            get_children(children_offsets, children_data, 0).iter().rev().map(|&c| (c as usize, false)).collect()
        };
        while let Some((node, exiting)) = stack.pop() {
            let f = name_ids[node] as u32;
            let parent = parents[node];
            let edge = (parent != usize::MAX).then(|| (name_ids[parent] as u32, f));
            if exiting {
                function_depth[f as usize] -= 1;
                if let Some(edge) = edge {
                    *edge_depth.get_mut(&edge).unwrap() -= 1;
                }
                continue;
            }

            let children = get_children(children_offsets, children_data, node);
            let child_sum: i64 = children.iter().map(|&c| values[c as usize]).sum();
            self_values[f as usize] += values[node] - child_sum;
            if function_depth[f as usize] == 0 {
                inclusive[f as usize] += values[node];
            }
            function_depth[f as usize] += 1;
            if let Some(edge) = edge {
                let depth = edge_depth.entry(edge).or_insert(0);
                if *depth == 0 {
                    *edge_weights.entry(edge).or_insert(0) += values[node];
                }
                *depth += 1;
            }

            stack.push((node, true));
            for &c in children.iter().rev() {
                parents[c as usize] = if node == 0 && !root_is_frame { usize::MAX } else { node };
                stack.push((c as usize, false));
            }
        }

        // Instances by function: a counting sort of the frame nodes by name id.
        let first = if root_is_frame { 0 } else { 1 };
        let mut instance_offsets = vec![0u32; string_count + 1];
        for &id in &name_ids[first..] {
            instance_offsets[id as usize + 1] += 1;
        }
        for f in 0..string_count {
            instance_offsets[f + 1] += instance_offsets[f];
        }
        let mut next = instance_offsets.clone();
        let mut instance_data = vec![0u32; node_count - first];
        for node in first..node_count {
            let f = name_ids[node] as usize;
            instance_data[next[f] as usize] = node as u32;
            next[f] += 1;
        }

        let mut edges: Vec<(u32, u32, i64)> = edge_weights.into_iter().map(|((f, g), w)| (f, g, w)).collect();
        edges.sort_unstable_by_key(|&(f, g, _)| (f, g));
        let (callee_offsets, callees) = adjacency(&edges, string_count, |&(f, g, w)| (f, g, w));
        edges.sort_unstable_by_key(|&(f, g, _)| (g, f));
        let (caller_offsets, callers) = adjacency(&edges, string_count, |&(f, g, w)| (g, f, w));

        CallGraph {
            self_values,
            inclusive,
            instance_offsets,
            instance_data,
            callee_offsets,
            callees,
            caller_offsets,
            callers,
        }
    }

    /// Samples in which function `f` is running, not one of its callees.
    pub fn self_value(&self, f: usize) -> i64 {
        self.self_values[f]
    }

    /// Samples in which function `f` is on the stack.
    pub fn inclusive(&self, f: usize) -> i64 {
        self.inclusive[f]
    }

    /// The stack graph nodes of function `f`, in node order. Empty for a name only a placeholder
    /// root has.
    pub fn instances(&self, f: usize) -> &[u32] {
        &self.instance_data[self.instance_offsets[f] as usize..self.instance_offsets[f + 1] as usize]
    }

    /// The functions `f` calls, by name id, with the samples each edge is on the stack for.
    pub fn callees(&self, f: usize) -> &[(u32, i64)] {
        &self.callees[self.callee_offsets[f] as usize..self.callee_offsets[f + 1] as usize]
    }

    /// The functions that call `f`, by name id, with the samples each edge is on the stack for.
    pub fn callers(&self, f: usize) -> &[(u32, i64)] {
        &self.callers[self.caller_offsets[f] as usize..self.caller_offsets[f + 1] as usize]
    }
}

/// Builds CSR adjacency lists from edges sorted by their `key(edge).0`, as `(offsets, entries)`
/// with entries `(key.1, key.2)`.
fn adjacency<E>(edges: &[E], string_count: usize, key: impl Fn(&E) -> (u32, u32, i64)) -> (Vec<u32>, Vec<(u32, i64)>) {
    let mut offsets = vec![0u32; string_count + 1];
    let mut entries = Vec::with_capacity(edges.len());
    for edge in edges {
        let (from, to, weight) = key(edge);
        offsets[from as usize + 1] += 1;
        entries.push((to, weight));
    }
    for f in 0..string_count {
        offsets[f + 1] += offsets[f];
    }
    (offsets, entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_stacks_impl, WasmStackGraph};

    fn call_graph(stacks: &str) -> (WasmStackGraph, CallGraph) {
        let g = parse_stacks_impl(stacks.as_bytes());
        let cg = CallGraph::new(
            &g.children_offsets,
            &g.children_data,
            &g.name_ids,
            g.names().string_count(),
            &g.values,
            false,
        );
        (g, cg)
    }

    fn id(g: &WasmStackGraph, name: &str) -> usize {
        g.names().find(name).unwrap_or_else(|| panic!("no function {name}"))
    }

    /// Edges by function name, sorted by name.
    fn named(g: &WasmStackGraph, edges: &[(u32, i64)]) -> Vec<(String, i64)> {
        let mut named: Vec<_> = edges.iter().map(|&(f, w)| (g.names().string(f as usize).to_string(), w)).collect();
        named.sort();
        named
    }

    #[test]
    fn test_functions_merge_across_call_paths() {
        let (g, cg) = call_graph("a;c;d 5\nb;c 3\nb;d 2\n");
        let c = id(&g, "c");
        assert_eq!((cg.self_value(c), cg.inclusive(c)), (3, 8));
        assert_eq!(cg.instances(c).len(), 2);
        let d = id(&g, "d");
        assert_eq!((cg.self_value(d), cg.inclusive(d)), (7, 7));
        assert_eq!(named(&g, cg.callers(d)), [("b".to_string(), 2), ("c".to_string(), 5)]);
        assert_eq!(named(&g, cg.callees(c)), [("d".to_string(), 5)]);
        // The root is not a caller.
        assert!(cg.callers(id(&g, "a")).is_empty());
    }

    #[test]
    fn test_recursion_counts_each_sample_once() {
        // f → f → f: one function with a self edge, counted once per sample.
        let (g, cg) = call_graph("a;f;f;f 4\na;f 1\n");
        let f = id(&g, "f");
        assert_eq!((cg.self_value(f), cg.inclusive(f)), (5, 5));
        assert_eq!(named(&g, cg.callees(f)), [("f".to_string(), 4)]);
        assert_eq!(named(&g, cg.callers(f)), [("a".to_string(), 5), ("f".to_string(), 4)]);
    }

    #[test]
    fn test_repeated_edges_count_once() {
        // f → g repeats in the stack (f → g → f → g); the edge is on the stack for 3 samples.
        let (g, cg) = call_graph("f;g;f;g 3\n");
        let (f, gf) = (id(&g, "f"), id(&g, "g"));
        assert_eq!(named(&g, cg.callees(f)), [("g".to_string(), 3)]);
        assert_eq!(named(&g, cg.callees(gf)), [("f".to_string(), 3)]);
        assert_eq!((cg.inclusive(f), cg.inclusive(gf)), (3, 3));
    }

    /// Compares against figures computed directly from each stack line, on generated stacks over
    /// a small alphabet, so recursion and mutual recursion are common: per line, the leaf gets the
    /// self samples, each distinct function on the line the inclusive samples, and each distinct
    /// adjacent caller → callee pair the edge samples.
    #[test]
    fn test_matches_brute_force_on_generated_stacks() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |bound: u64| {
            // xorshift64*
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            seed.wrapping_mul(0x2545_f491_4f6c_dd1d) % bound
        };
        for _ in 0..300 {
            let mut text = String::new();
            let mut self_values: AHashMap<String, i64> = AHashMap::new();
            let mut inclusive: AHashMap<String, i64> = AHashMap::new();
            let mut edges: AHashMap<(String, String), i64> = AHashMap::new();
            for _ in 0..1 + next(12) {
                let frames: Vec<String> = (0..1 + next(8)).map(|_| ((b'a' + next(5) as u8) as char).to_string()).collect();
                let count = 1 + next(9) as i64;
                text.push_str(&format!("{} {count}\n", frames.join(";")));
                *self_values.entry(frames.last().unwrap().clone()).or_insert(0) += count;
                let distinct: ahash::AHashSet<&String> = frames.iter().collect();
                for f in distinct {
                    *inclusive.entry(f.clone()).or_insert(0) += count;
                }
                let pairs: ahash::AHashSet<(&String, &String)> = frames.windows(2).map(|w| (&w[0], &w[1])).collect();
                for (f, g) in pairs {
                    *edges.entry((f.clone(), g.clone())).or_insert(0) += count;
                }
            }

            let (g, cg) = call_graph(&text);
            for f in 0..g.names().string_count() {
                let name = g.names().string(f);
                if cg.instances(f).is_empty() {
                    continue; // the root
                }
                assert_eq!(cg.self_value(f), *self_values.get(name).unwrap_or(&0), "self of {name} in\n{text}");
                assert_eq!(cg.inclusive(f), inclusive[name], "inclusive of {name} in\n{text}");
                for &(callee, w) in cg.callees(f) {
                    let callee = g.names().string(callee as usize).to_string();
                    assert_eq!(w, edges[&(name.to_string(), callee.clone())], "edge {name} -> {callee} in\n{text}");
                }
                let callers: i64 = cg.callers(f).len() as i64;
                let expected_callers = edges.keys().filter(|(_, g)| g == name).count() as i64;
                assert_eq!(callers, expected_callers, "callers of {name} in\n{text}");
            }
            let edge_count: usize = (0..g.names().string_count()).map(|f| cg.callees(f).len()).sum();
            assert_eq!(edge_count, edges.len(), "edges in\n{text}");
        }
    }

    /// Validation against a real recording: set `JFR_TEST_FILE` to a .jfr path and run
    /// `cargo test --release -- --ignored --nocapture external_call_graph`. Checks every
    /// function's inclusive value against `aggregate`'s merged root, and that self values add up
    /// to the total.
    #[test]
    #[ignore = "requires JFR_TEST_FILE env var pointing at a real .jfr"]
    fn test_external_call_graph() {
        let path = std::env::var("JFR_TEST_FILE").expect("set JFR_TEST_FILE to a .jfr path");
        let data = std::fs::read(&path).expect("failed to read JFR_TEST_FILE");
        for (kind, g) in crate::jfr::parse_jfr_impl(&data).expect("failed to decode") {
            let start = std::time::Instant::now();
            let cg = CallGraph::new(&g.children_offsets, &g.children_data, &g.name_ids, g.names().string_count(), &g.values, false);
            let elapsed = start.elapsed();
            let functions: Vec<usize> = (0..g.names().string_count()).filter(|&f| !cg.instances(f).is_empty()).collect();
            let edges: usize = functions.iter().map(|&f| cg.callees(f).len()).sum();
            println!("{kind}: {} nodes, {} functions, {edges} edges, built in {elapsed:?}", g.node_count(), functions.len());

            let self_total: i64 = functions.iter().map(|&f| cg.self_value(f)).sum();
            let root_self = g.values[0] - crate::get_children(&g.children_offsets, &g.children_data, 0).iter().map(|&c| g.values[c as usize]).sum::<i64>();
            assert_eq!(self_total + root_self, g.values[0]);
            for &f in &functions {
                let merged = crate::wasm_merge_children(
                    &g.children_offsets, &g.children_data, &g.name_ids, &g.names_data, &g.names_offsets, &g.values,
                    g.names().string(f),
                )
                .unwrap();
                assert_eq!(cg.inclusive(f), merged.values[0], "{}", g.names().string(f));
            }
        }
    }

    #[test]
    fn test_a_merged_graph_root_is_a_function() {
        let g = parse_stacks_impl(b"a;f;x 3\nb;f;g;x 2\n");
        let merged = crate::wasm_merge_children(
            &g.children_offsets,
            &g.children_data,
            &g.name_ids,
            &g.names_data,
            &g.names_offsets,
            &g.values,
            "f",
        )
        .unwrap();
        let cg = CallGraph::new(
            &merged.children_offsets,
            &merged.children_data,
            &merged.name_ids,
            merged.names().string_count(),
            &merged.values,
            true,
        );
        let f = id(&merged, "f");
        assert_eq!((cg.self_value(f), cg.inclusive(f)), (0, 5));
        assert_eq!(cg.instances(f), [0]);
        assert_eq!(named(&merged, cg.callees(f)), [("g".to_string(), 2), ("x".to_string(), 3)]);
        assert_eq!(named(&merged, cg.callers(id(&merged, "x"))), [("f".to_string(), 3), ("g".to_string(), 2)]);
    }

    #[test]
    fn test_inclusive_matches_merged_root() {
        // aggregate's merged root counts each stack through f once; so does inclusive.
        let (g, cg) = call_graph("a;f;b;f 3\nc;f 2\nf 1\n");
        let f = id(&g, "f");
        let merged = crate::wasm_merge_children(
            &g.children_offsets,
            &g.children_data,
            &g.name_ids,
            &g.names_data,
            &g.names_offsets,
            &g.values,
            "f",
        )
        .unwrap();
        assert_eq!(cg.inclusive(f), merged.values[0]);
        assert_eq!(cg.inclusive(f), 6);
    }
}
