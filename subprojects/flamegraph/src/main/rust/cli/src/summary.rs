// Budget-limited weighted-tree summarization shared by the subtree, callers and calls commands.

use crate::table::Table;

/// Abstract weighted tree that forward (subtree), reverse (callers) and call-graph (calls) views
/// implement. The budget-limited tree builder and box-drawing renderer operate on this interface.
pub trait WeightedTree {
    type Node;

    /// The line's text: its ID and name.
    fn label(&self, node: &Self::Node) -> String;
    /// The figures in the line's columns.
    fn cells(&self, node: &Self::Node) -> Vec<String>;
    fn weight(&self, node: &Self::Node) -> i64;
    fn child_count(&self, node: &Self::Node) -> usize;
    /// Returns children sorted by weight descending.
    fn sorted_children(&self, node: &Self::Node) -> Vec<Self::Node>;
    /// Text printed after the label, if any.
    fn note(&self, _node: &Self::Node) -> Option<String> {
        None
    }
}

#[derive(Debug)]
pub struct SummaryNode {
    pub label: String,
    pub cells: Vec<String>,
    pub children: Vec<SummaryNode>,
    /// Printed after the label.
    pub note: Option<String>,
    /// Children with samples that the line budget left out.
    pub hidden: usize,
}

impl SummaryNode {
    /// Lines with children the line budget left out.
    pub fn cut_lines(&self) -> usize {
        usize::from(self.hidden > 0) + self.children.iter().map(SummaryNode::cut_lines).sum::<usize>()
    }
}

/// Budget-limited tree builder. Returns actual lines used so unused budget flows to siblings.
/// With `collapse`, a pass-through spine (single children carrying their parent's entire weight)
/// of two or more frames is elided to one line, so a deep wrapper chain doesn't consume the whole
/// line budget.
pub fn build_tree<T: WeightedTree>(view: &T, root: T::Node, budget: i64, collapse: bool) -> SummaryNode {
    fn visit<T: WeightedTree>(view: &T, node: T::Node, budget: i64, collapse: bool) -> SummaryNode {
        let value = view.weight(&node);
        let child_count = view.child_count(&node);

        let mut result = SummaryNode {
            label: view.label(&node),
            cells: view.cells(&node),
            children: Vec::new(),
            note: view.note(&node),
            hidden: 0,
        };

        if child_count == 0 {
            return result;
        }
        if budget <= 1 {
            result.hidden = view.sorted_children(&node).iter().filter(|c| view.weight(c) != 0).count();
            return result;
        }

        if collapse {
            // Walk the pass-through spine below this node: while a node has exactly one child
            // and that child carries the node's entire weight, the child line adds no
            // information. `tail` ends on the last equal-weight node: the first that branches,
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
            // Frames strictly between node and tail. Eliding one would take the line it frees.
            let skipped = spine_length - 2;
            if skipped >= 2 {
                let tail = tail.unwrap();
                let shown = budget >= 3;
                result.children.push(SummaryNode {
                    label: format!("... {skipped} frames with the same total ..."),
                    cells: Vec::new(),
                    children: if shown { vec![visit(view, tail, budget - 2, collapse)] } else { vec![] },
                    note: None,
                    hidden: usize::from(!shown),
                });
                return result;
            }
        }

        // Shares are of the children's combined weight. For a stack graph that is the node's
        // weight minus its self weight, but a diff's children can outweigh their parent (changes
        // that cancel out), and a root whose changes cancel entirely still shows its children.
        let children = view.sorted_children(&node);
        let mut weight_left: f64 = children.iter().map(|c| view.weight(c) as f64).sum();
        let mut budget_left = budget - 1;
        for child in children {
            if budget_left <= 0 || weight_left <= 0.0 {
                result.hidden += usize::from(view.weight(&child) != 0);
                continue;
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

/// Adds a SummaryNode tree to `table` as rows, its shape drawn with box-drawing characters.
pub fn render_tree(node: &SummaryNode, table: &mut Table) {
    fn visit(node: &SummaryNode, own_prefix: &str, child_prefix: &str, table: &mut Table) {
        let note = node.note.as_ref().map(|n| format!("  {n}")).unwrap_or_default();
        table.row(node.cells.clone(), format!("{own_prefix}{}{note}", node.label));
        for (i, child) in node.children.iter().enumerate() {
            let is_last = i == node.children.len() - 1;
            visit(
                child,
                &format!("{child_prefix}{}", if is_last { "└─ " } else { "├─ " }),
                &format!("{child_prefix}{}", if is_last { "   " } else { "│  " }),
                table,
            );
        }
    }
    visit(node, "", "", table);
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

        fn label(&self, n: &Self::Node) -> String {
            n.name.to_string()
        }
        fn cells(&self, n: &Self::Node) -> Vec<String> {
            let self_value = n.value - n.children.iter().map(|c| c.value).sum::<i64>();
            vec![n.value.to_string(), self_value.to_string()]
        }
        fn weight(&self, n: &Self::Node) -> i64 {
            n.value
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
        let mut table = Table::new(["total", "self"]);
        render_tree(&build_tree(&&MockView, root, budget, collapse), &mut table);
        table.render(false)
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
    fn lines_whose_children_the_budget_cut_are_counted() {
        let tree = node("root", 10, vec![node("a", 5, vec![node("b", 5, vec![])]), node("c", 5, vec![node("d", 5, vec![])])]);
        assert_eq!(build_tree(&&MockView, &tree, 5, false).cut_lines(), 0);
        // root, a, c: b is cut from a, d from c.
        assert_eq!(build_tree(&&MockView, &tree, 3, false).cut_lines(), 2);
        // root alone.
        assert_eq!(build_tree(&&MockView, &tree, 1, false).cut_lines(), 1);
    }

    #[test]
    fn spine_is_elided_to_one_line_budget_reaches_the_branches() {
        assert_eq!(render(&spine_tree(), 10, true), [
            "100   0  root",
            "         └─ ... 3 frames with the same total ...",
            "100   0     └─ branch",
            " 60  60        ├─ e",
            " 40  40        └─ f",
        ]);
    }

    #[test]
    fn without_collapse_the_spine_consumes_the_budget() {
        assert_eq!(render(&spine_tree(), 5, false), [
            "100  0  root",
            "100  0  └─ a",
            "100  0     └─ b",
            "100  0        └─ c",
            "100  0           └─ branch",
        ]);
    }

    #[test]
    fn a_spine_of_two_renders_the_child_directly_with_no_elision() {
        let tree = node("root", 100, vec![node("only", 100, vec![node("e", 60, vec![]), node("f", 40, vec![])])]);
        let lines = render(&tree, 10, true);
        assert!(lines[1].ends_with("└─ only"));
        assert!(!lines.join("\n").contains("frames with"));
    }

    #[test]
    fn a_single_frame_is_shown_rather_than_elided() {
        let tree = node("root", 100, vec![node("a", 100, vec![node(
            "branch",
            100,
            vec![node("e", 60, vec![]), node("f", 40, vec![])],
        )])]);
        let lines = render(&tree, 10, true);
        assert!(lines[1].ends_with("└─ a"), "{lines:?}");
        assert!(lines[2].ends_with("└─ branch"), "{lines:?}");
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
        assert_eq!(lines[0], "100  40  root");
        // The spine below a still collapses (a -> b -> c -> d, equal totals).
        assert!(lines[1].ends_with("└─ a"));
        assert!(lines[2].ends_with("... 2 frames with the same total ..."));
        assert!(lines[3].ends_with("└─ d"));
    }

    #[test]
    fn budget_of_two_shows_the_elision_marker_but_not_the_tail() {
        let lines = render(&spine_tree(), 2, true);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("└─ ... 3 frames with the same total ..."));
    }
}
