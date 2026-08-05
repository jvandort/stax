// Budget-limited weighted-tree summarization shared by the MCP server's
// subtree_summary and callers tools. Kept free of mcp.ts's module side
// effects (wasm init, stdio loop) so it can be unit tested.

export function formatSamples(n: bigint, total: bigint): string {
    const pct = total > 0n ? (Number(n) / Number(total) * 100).toFixed(1) : "0.0"
    return `${n.toLocaleString()} samples (${pct}%)`
}

/**
 * Abstract weighted tree that both forward (subtree) and reverse (caller) views implement.
 * The budget-limited tree builder and box-drawing renderer operate on this interface.
 */
export interface WeightedTree<N> {
    displayName(node: N): string
    weight(node: N): bigint
    selfWeight(node: N): bigint
    childCount(node: N): number
    /** Returns children sorted by weight descending. */
    sortedChildren(node: N): N[]
    /** Optional node ID for display (e.g. forward graph has IDs, caller graph doesn't). */
    nodeId?(node: N): number | undefined
}

export interface SummaryNode {
    nodeId?: number
    displayName: string
    value: bigint
    selfValue: bigint
    totalChildren: number
    children: SummaryNode[]
    /** True for a synthetic "... N frames with same total ..." elision line. */
    isElision?: boolean
}

/**
 * Budget-limited tree builder. Returns actual lines used so unused budget
 * flows to siblings. With `collapse`, a pass-through spine — single children
 * carrying their parent's entire weight — is elided to one line (mirroring
 * hot_path), so a deep wrapper chain doesn't consume the whole line budget.
 */
export function buildTree<N>(view: WeightedTree<N>, root: N, budget: number, collapse = false): SummaryNode {
    function visit(node: N, budget: number): SummaryNode {
        const value = view.weight(node)
        const selfValue = view.selfWeight(node)
        const totalChildren = view.childCount(node)

        const result: SummaryNode = {
            nodeId: view.nodeId?.(node),
            displayName: view.displayName(node),
            value,
            selfValue,
            totalChildren,
            children: [],
        }

        if (totalChildren === 0 || budget <= 1) return result

        if (collapse) {
            // Walk the pass-through spine below this node: while a node has
            // exactly one child and that child carries the node's entire
            // weight, the child line adds no information. `tail` ends on the
            // last equal-weight node — the first that branches, has self
            // samples, or is a leaf.
            let tail = node
            let spineLength = 1
            while (view.childCount(tail) === 1) {
                const child = view.sortedChildren(tail)[0]!
                if (view.weight(child) !== value) break
                tail = child
                spineLength++
            }
            const skipped = spineLength - 2 // frames strictly between node and tail
            if (skipped >= 1) {
                result.children.push({
                    displayName: `... ${skipped} frame${skipped === 1 ? "" : "s"} with same total ...`,
                    value,
                    selfValue: 0n,
                    totalChildren: 0,
                    children: budget >= 3 ? [visit(tail, budget - 2)] : [],
                    isElision: true,
                })
                return result
            }
            // A spine of two (skipped == 0) needs no elision: the child loop
            // below renders the single child directly.
        }

        const sorted = view.sortedChildren(node)
        let weightLeft = Number(value - selfValue)
        let budgetLeft = budget - 1
        for (const child of sorted) {
            if (budgetLeft <= 0 || weightLeft <= 0) break
            const childWeight = Number(view.weight(child))
            const alloc = Math.ceil(childWeight / weightLeft * budgetLeft)
            weightLeft -= childWeight
            if (alloc === 0) continue
            const childNode = visit(child, alloc)
            budgetLeft -= countNodes(childNode)
            result.children.push(childNode)
        }

        return result
    }

    return visit(root, budget)
}

function countNodes(node: SummaryNode): number {
    let count = 1
    for (const child of node.children) count += countNodes(child)
    return count
}

/** Render a SummaryNode tree with box-drawing. childLabel is "children" or "callers". */
export function renderTree(node: SummaryNode, total: bigint, childLabel: string, ownPrefix: string, childPrefix: string, lines: string[]): void {
    if (node.isElision) {
        lines.push(`${ownPrefix}${node.displayName}`)
    } else {
        const selfStr = node.selfValue > 0n ? `, self: ${formatSamples(node.selfValue, total)}` : ""
        const childStr = node.totalChildren > 0 ? `, ${childLabel}: ${node.totalChildren}` : ""
        const idStr = node.nodeId != null ? `[${node.nodeId}] ` : ""
        lines.push(`${ownPrefix}${idStr}${node.displayName}, ${formatSamples(node.value, total)}${selfStr}${childStr}`)
    }
    for (let i = 0; i < node.children.length; i++) {
        const isLast = i === node.children.length - 1
        renderTree(
            node.children[i]!,
            total,
            childLabel,
            childPrefix + (isLast ? "└─ " : "├─ "),
            childPrefix + (isLast ? "   " : "│  "),
            lines,
        )
    }
}
