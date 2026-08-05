import { describe, expect, test } from "vitest"
import {
    buildTree,
    renderTree,
    type WeightedTree,
} from "../../main/ts/summaryTree.ts"

interface MockNode {
    name: string
    value: bigint
    children: MockNode[]
}

const node = (
    name: string,
    value: number,
    children: MockNode[] = [],
): MockNode => ({ name, value: BigInt(value), children })

const view: WeightedTree<MockNode> = {
    displayName: (n) => n.name,
    weight: (n) => n.value,
    selfWeight: (n) =>
        n.value - n.children.reduce((sum, c) => sum + c.value, 0n),
    childCount: (n) => n.children.length,
    sortedChildren: (n) =>
        [...n.children].sort((a, b) => Number(b.value - a.value)),
}

const render = (root: MockNode, budget: number, collapse: boolean): string[] => {
    const lines: string[] = []
    renderTree(buildTree(view, root, budget, collapse), root.value, "children", "", "", lines)
    return lines
}

/** root -> a -> b -> c -> branch(e, f): a pass-through spine of equal totals. */
const spineTree = node("root", 100, [
    node("a", 100, [
        node("b", 100, [
            node("c", 100, [
                node("branch", 100, [node("e", 60), node("f", 40)]),
            ]),
        ]),
    ]),
])

describe("pass-through spine collapsing", () => {
    test("spine is elided to one line, budget reaches the branches", () => {
        const lines = render(spineTree, 10, true)
        expect(lines).toEqual([
            "root, 100 samples (100.0%), children: 1",
            "└─ ... 3 frames with same total ...",
            "   └─ branch, 100 samples (100.0%), children: 2",
            "      ├─ e, 60 samples (60.0%), self: 60 samples (60.0%)",
            "      └─ f, 40 samples (40.0%), self: 40 samples (40.0%)",
        ])
    })

    test("without collapse the spine consumes the budget", () => {
        const lines = render(spineTree, 5, false)
        expect(lines).toEqual([
            "root, 100 samples (100.0%), children: 1",
            "└─ a, 100 samples (100.0%), children: 1",
            "   └─ b, 100 samples (100.0%), children: 1",
            "      └─ c, 100 samples (100.0%), children: 1",
            "         └─ branch, 100 samples (100.0%), children: 2",
        ])
    })

    test("a spine of two renders the child directly with no elision", () => {
        const tree = node("root", 100, [node("only", 100, [node("e", 60), node("f", 40)])])
        const lines = render(tree, 10, true)
        expect(lines[1]).toContain("only")
        expect(lines.join("\n")).not.toContain("frames with same total")
    })

    test("a weight drop breaks the spine", () => {
        // root has 40 self samples, so its child does not carry the full weight.
        const tree = node("root", 100, [node("a", 60, [node("b", 60, [node("c", 60, [node("d", 60)])])])])
        const lines = render(tree, 10, true)
        expect(lines[0]).toContain("root")
        expect(lines[0]).toContain("self: 40 samples")
        // The spine below a still collapses (a -> b -> c -> d, equal totals).
        expect(lines[1]).toContain("a, 60 samples")
        expect(lines[2]).toContain("... 2 frames with same total ...")
        expect(lines[3]).toContain("d, 60 samples")
    })

    test("budget of two shows the elision marker but not the tail", () => {
        const lines = render(spineTree, 2, true)
        expect(lines).toEqual([
            "root, 100 samples (100.0%), children: 1",
            "└─ ... 3 frames with same total ...",
        ])
    })

    test("single elided frame uses singular wording", () => {
        const tree = node("root", 100, [
            node("a", 100, [node("branch", 100, [node("e", 60), node("f", 40)])]),
        ])
        const lines = render(tree, 10, true)
        expect(lines[1]).toBe("└─ ... 1 frame with same total ...")
    })
})
