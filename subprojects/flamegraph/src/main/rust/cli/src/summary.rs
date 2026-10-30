// Budget-limited weighted-tree summarization shared by the subtree and callers commands.

use crate::graph::format_samples;

/// Abstract weighted tree that both forward (subtree) and reverse (caller) views implement.
/// The budget-limited tree builder and box-drawing renderer operate on this interface.
pub trait WeightedTree {
    type Node;

    fn display_name(&self, node: &Self::Node) -> String;
    fn weight(&self, node: &Self::Node) -> i64;
    fn self_weight(&self, node: &Self::Node) -> i64;
    fn child_count(&self, node: &Self::Node) -> usize;
    /// Returns children sorted by weight descending.
    fn sorted_children(&self, node: &Self::Node) -> Vec<Self::Node>;
    /// Node ID for display (the forward graph has IDs, the caller graph doesn't).
    fn node_id(&self, _node: &Self::Node) -> Option<usize> {
        None
    }
}

#[derive(Debug)]
pub struct SummaryNode {
    pub node_id: Option<usize>,
    pub display_name: String,
    pub value: i64,
    pub self_value: i64,
    pub total_children: usize,
    pub children: Vec<SummaryNode>,
    /// True for a synthetic "... N frames with same total ..." elision line.
    pub is_elision: bool,
}

/// Budget-limited tree builder. Returns actual lines used so unused budget flows to siblings.
/// With `collapse`, a pass-through spine — single children carrying their parent's entire
/// weight — is elided to one line (mirroring hot-path), so a deep wrapper chain doesn't consume
/// the whole line budget.
pub fn build_tree<T: WeightedTree>(view: &T, root: T::Node, budget: i64, collapse: bool) -> SummaryNode {
    fn visit<T: WeightedTree>(view: &T, node: T::Node, budget: i64, collapse: bool) -> SummaryNode {
        let value = view.weight(&node);
        let self_value = view.self_weight(&node);
        let total_children = view.child_count(&node);

        let mut result = SummaryNode {
            node_id: view.node_id(&node),
            display_name: view.display_name(&node),
            value,
            self_value,
            total_children,
            children: Vec::new(),
            is_elision: false,
        };

        if total_children == 0 || budget <= 1 {
            return result;
        }

        if collapse {
            // Walk the pass-through spine below this node: while a node has exactly one child
            // and that child carries the node's entire weight, the child line adds no
            // information. `tail` ends on the last equal-weight node — the first that branches,
            // has self samples, or is a leaf.
            let mut tail: Option<T::Node> = None;
            let mut spine_length = 1;
            loop {
                let current = tail.as_ref().unwrap_or(&node);
                if view.child_count(current) != 1 {
                    break;
                }
                let child = view.sorted_children(current).into_iter().next().unwrap();
                if view.weight(&child) != value {
                    break;
                }
                tail = Some(child);
                spine_length += 1;
            }
            let skipped = spine_length - 2; // frames strictly between node and tail
            if skipped >= 1 {
                let tail = tail.unwrap();
                result.children.push(SummaryNode {
                    node_id: None,
                    display_name: format!(
                        "... {skipped} frame{} with same total ...",
                        if skipped == 1 { "" } else { "s" }
                    ),
                    value,
                    self_value: 0,
                    total_children: 0,
                    children: if budget >= 3 { vec![visit(view, tail, budget - 2, collapse)] } else { vec![] },
                    is_elision: true,
                });
                return result;
            }
            // A spine of two (skipped == 0) needs no elision: the child loop below renders the
            // single child directly.
        }

        let mut weight_left = (value - self_value) as f64;
        let mut budget_left = budget - 1;
        for child in view.sorted_children(&node) {
            if budget_left <= 0 || weight_left <= 0.0 {
                break;
            }
            let child_weight = view.weight(&child) as f64;
            let alloc = (child_weight / weight_left * budget_left as f64).ceil() as i64;
            weight_left -= child_weight;
            if alloc == 0 {
                continue;
            }
            let child_node = visit(view, child, alloc, collapse);
            budget_left -= count_nodes(&child_node);
            result.children.push(child_node);
        }

        result
    }

    visit(view, root, budget, collapse)
}

fn count_nodes(node: &SummaryNode) -> i64 {
    1 + node.children.iter().map(count_nodes).sum::<i64>()
}

/// Render a SummaryNode tree with box-drawing. `child_label` is "children" or "callers".
pub fn render_tree(
    node: &SummaryNode,
    total: i64,
    child_label: &str,
    own_prefix: &str,
    child_prefix: &str,
    lines: &mut Vec<String>,
) {
    if node.is_elision {
        lines.push(format!("{own_prefix}{}", node.display_name));
    } else {
        let self_str = if node.self_value > 0 {
            format!(", self: {}", format_samples(node.self_value, total))
        } else {
            String::new()
        };
        let child_str = if node.total_children > 0 {
            format!(", {child_label}: {}", node.total_children)
        } else {
            String::new()
        };
        let id_str = node.node_id.map(|id| format!("[{id}] ")).unwrap_or_default();
        lines.push(format!(
            "{own_prefix}{id_str}{}, {}{self_str}{child_str}",
            node.display_name,
            format_samples(node.value, total)
        ));
    }
    for (i, child) in node.children.iter().enumerate() {
        let is_last = i == node.children.len() - 1;
        render_tree(
            child,
            total,
            child_label,
            &format!("{child_prefix}{}", if is_last { "└─ " } else { "├─ " }),
            &format!("{child_prefix}{}", if is_last { "   " } else { "│  " }),
            lines,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockNode {
        name: &'static str,
        value: i64,
        children: Vec<MockNode>,
    }

    fn node(name: &'static str, value: i64, children: Vec<MockNode>) -> MockNode {
        MockNode { name, value, children }
    }

    struct MockView;

    impl<'a> WeightedTree for &'a MockView {
        type Node = &'a MockNode;

        fn display_name(&self, n: &Self::Node) -> String {
            n.name.to_string()
        }
        fn weight(&self, n: &Self::Node) -> i64 {
            n.value
        }
        fn self_weight(&self, n: &Self::Node) -> i64 {
            n.value - n.children.iter().map(|c| c.value).sum::<i64>()
        }
        fn child_count(&self, n: &Self::Node) -> usize {
            n.children.len()
        }
        fn sorted_children(&self, n: &Self::Node) -> Vec<Self::Node> {
            let mut children: Vec<_> = n.children.iter().collect();
            children.sort_by(|a, b| b.value.cmp(&a.value));
            children
        }
    }

    fn render(root: &MockNode, budget: i64, collapse: bool) -> Vec<String> {
        let mut lines = Vec::new();
        render_tree(&build_tree(&&MockView, root, budget, collapse), root.value, "children", "", "", &mut lines);
        lines
    }

    /// root -> a -> b -> c -> branch(e, f): a pass-through spine of equal totals.
    fn spine_tree() -> MockNode {
        node("root", 100, vec![node("a", 100, vec![node("b", 100, vec![node("c", 100, vec![node(
            "branch",
            100,
            vec![node("e", 60, vec![]), node("f", 40, vec![])],
        )])])])])
    }

    #[test]
    fn spine_is_elided_to_one_line_budget_reaches_the_branches() {
        assert_eq!(render(&spine_tree(), 10, true), [
            "root, 100 samples (100.0%), children: 1",
            "└─ ... 3 frames with same total ...",
            "   └─ branch, 100 samples (100.0%), children: 2",
            "      ├─ e, 60 samples (60.0%), self: 60 samples (60.0%)",
            "      └─ f, 40 samples (40.0%), self: 40 samples (40.0%)",
        ]);
    }

    #[test]
    fn without_collapse_the_spine_consumes_the_budget() {
        assert_eq!(render(&spine_tree(), 5, false), [
            "root, 100 samples (100.0%), children: 1",
            "└─ a, 100 samples (100.0%), children: 1",
            "   └─ b, 100 samples (100.0%), children: 1",
            "      └─ c, 100 samples (100.0%), children: 1",
            "         └─ branch, 100 samples (100.0%), children: 2",
        ]);
    }

    #[test]
    fn a_spine_of_two_renders_the_child_directly_with_no_elision() {
        let tree = node("root", 100, vec![node("only", 100, vec![node("e", 60, vec![]), node("f", 40, vec![])])]);
        let lines = render(&tree, 10, true);
        assert!(lines[1].contains("only"));
        assert!(!lines.join("\n").contains("frames with same total"));
    }

    #[test]
    fn a_weight_drop_breaks_the_spine() {
        // root has 40 self samples, so its child does not carry the full weight.
        let tree = node("root", 100, vec![node("a", 60, vec![node("b", 60, vec![node("c", 60, vec![node(
            "d",
            60,
            vec![],
        )])])])]);
        let lines = render(&tree, 10, true);
        assert!(lines[0].contains("root"));
        assert!(lines[0].contains("self: 40 samples"));
        // The spine below a still collapses (a -> b -> c -> d, equal totals).
        assert!(lines[1].contains("a, 60 samples"));
        assert!(lines[2].contains("... 2 frames with same total ..."));
        assert!(lines[3].contains("d, 60 samples"));
    }

    #[test]
    fn budget_of_two_shows_the_elision_marker_but_not_the_tail() {
        assert_eq!(render(&spine_tree(), 2, true), [
            "root, 100 samples (100.0%), children: 1",
            "└─ ... 3 frames with same total ...",
        ]);
    }

    #[test]
    fn single_elided_frame_uses_singular_wording() {
        let tree = node("root", 100, vec![node("a", 100, vec![node(
            "branch",
            100,
            vec![node("e", 60, vec![]), node("f", 40, vec![])],
        )])]);
        assert_eq!(render(&tree, 10, true)[1], "└─ ... 1 frame with same total ...");
    }
}
