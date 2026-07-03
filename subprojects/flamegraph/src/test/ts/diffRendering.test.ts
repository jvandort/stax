import { describe, expect, test } from "vitest"
import { DiffGraph, type StackGraphData } from "../../main/ts/stackGraph"
import {
    DEFAULT_DIFF_RENDER_OPTIONS,
    type DiffRenderOptions,
    getFullWidthChild,
    getMeasure,
    getSameWidthChain,
    hasRenderableContent,
    nodeDetails,
} from "../../main/ts/FlamegraphNode"

/**
 * Fixture exercising the tricky diff-rendering scenarios:
 *
 * Root      A=10000 B=10353  Δ=+353
 * ├─ Steady A=4000  B=4000   Δ=0     real ±800 swap that nets to zero
 * │   ├─ OldImpl   3000→2200 Δ=−800
 * │   └─ NewImpl   1000→1800 Δ=+800
 * ├─ Noisy  A=2500  B=2503   Δ=+3    leaf jitter that doesn't fully cancel
 * │   ├─ work      2493→2498 Δ=+5
 * │   ├─ tailA        7→0    Δ=−7
 * │   └─ tailB        0→5    Δ=+5
 * ├─ Mixed  A=2000  B=2500   Δ=+500  partial cancellation + self-time change
 * │   ├─ parse     1200→1900 Δ=+700
 * │   ├─ cache      600→350  Δ=−250
 * │   └─ (self)     200→250  Δ=+50
 * └─ IO     A=1500  B=1350   Δ=−150  net improvement hiding a new frame
 *     ├─ read      1500→950  Δ=−550
 *     └─ mmap         0→400  Δ=+400 (new in B)
 */
const NODE = {
    Root: 0,
    Steady: 1,
    OldImpl: 2,
    NewImpl: 3,
    Noisy: 4,
    work: 5,
    tailA: 6,
    tailB: 7,
    Mixed: 8,
    parse: 9,
    cache: 10,
    IO: 11,
    read: 12,
    mmap: 13,
} as const
const NAMES = Object.keys(NODE)
const CHILDREN: number[][] = [
    [1, 4, 8, 11],
    [2, 3],
    [],
    [],
    [5, 6, 7],
    [],
    [],
    [],
    [9, 10],
    [],
    [],
    [12, 13],
    [],
    [],
]
const A = [
    10000, 4000, 3000, 1000, 2500, 2493, 7, 0, 2000, 1200, 600, 1500, 1500, 0,
]
const B = [
    10353, 4000, 2200, 1800, 2503, 2498, 0, 5, 2500, 1900, 350, 1350, 950, 400,
]

const buildGraphData = (): StackGraphData => {
    const n = NAMES.length
    const encoder = new TextEncoder()
    const namesOffsets = new Int32Array(n + 1)
    const nameBytes: number[] = []
    NAMES.forEach((name, i) => {
        namesOffsets[i] = nameBytes.length
        nameBytes.push(...encoder.encode(name))
    })
    namesOffsets[n] = nameBytes.length
    const namesData = new Uint8Array(nameBytes)

    const childrenOffsets = new Int32Array(n + 1)
    const childrenFlat: number[] = []
    CHILDREN.forEach((cs, i) => {
        childrenOffsets[i] = childrenFlat.length
        childrenFlat.push(...cs)
    })
    childrenOffsets[n] = childrenFlat.length

    return {
        childrenOffsets,
        childrenData: new Int32Array(childrenFlat),
        namesData,
        namesOffsets,
        // Graph A's sample counts ride in the merged graph's values array.
        values: new BigInt64Array(A.map(BigInt)),
        displayNamesData: namesData,
        displayNamesOffsets: namesOffsets,
    }
}

const graph = new DiffGraph({
    graph: buildGraphData(),
    diffValues: new BigInt64Array(B.map(BigInt)),
})

const opts = (o: Partial<DiffRenderOptions>): DiffRenderOptions => ({
    ...DEFAULT_DIFF_RENDER_OPTIONS,
    ...o,
})

const delta = opts({ mode: "delta" })
const slower = opts({ mode: "regressions" })
const faster = opts({ mode: "improvements" })

describe("DiffGraph derived arrays", () => {
    test("deltas are b - a", () => {
        expect([...graph.deltas].map(Number)).toEqual([
            353, 0, -800, 800, 3, 5, -7, 5, 500, 700, -250, -150, -550, 400,
        ])
    })

    test("selfDeltas subtract child deltas", () => {
        expect([...graph.selfDeltas].map(Number)).toEqual([
            0, 0, -800, 800, 0, 5, -7, 5, 50, 700, -250, 0, -550, 400,
        ])
    })
})

describe("delta mode", () => {
    test("width is |net delta|", () => {
        expect(getMeasure(graph, NODE.Root, delta)).toBe(353n)
        expect(getMeasure(graph, NODE.Mixed, delta)).toBe(500n)
        expect(getMeasure(graph, NODE.IO, delta)).toBe(150n)
    })

    test("a subtree whose changes cancel gets zero width", () => {
        expect(getMeasure(graph, NODE.Steady, delta)).toBe(0n)
        expect(hasRenderableContent(graph, NODE.Steady, delta)).toBe(false)
    })
})

describe("signed views (net semantics)", () => {
    test("Slower keeps only net-slower subtrees", () => {
        expect(getMeasure(graph, NODE.Root, slower)).toBe(353n)
        expect(getMeasure(graph, NODE.Mixed, slower)).toBe(500n)
        expect(getMeasure(graph, NODE.IO, slower)).toBe(0n)
    })

    test("Faster renders under a net-slower root via the container rule", () => {
        // The root's own measure is 0 (it got net slower)...
        expect(getMeasure(graph, NODE.Root, faster)).toBe(0n)
        // ...but the view still renders, with IO filling it.
        expect(hasRenderableContent(graph, NODE.Root, faster)).toBe(true)
        expect(getMeasure(graph, NODE.IO, faster)).toBe(150n)
        // mmap got slower, so it is correctly absent from Faster.
        expect(getMeasure(graph, NODE.mmap, faster)).toBe(0n)
    })

    test("drilling into a net-faster subtree reveals slowdowns inside it", () => {
        // At the top level, Slower hides IO entirely (net −150)...
        expect(getMeasure(graph, NODE.IO, slower)).toBe(0n)
        // ...but rooted at IO, the container rule exposes mmap (+400).
        expect(hasRenderableContent(graph, NODE.IO, slower)).toBe(true)
        expect(getMeasure(graph, NODE.mmap, slower)).toBe(400n)
    })

    test("a leaf with nothing below is not a container", () => {
        expect(hasRenderableContent(graph, NODE.NewImpl, faster)).toBe(false)
    })
})

describe("swap", () => {
    test("delta widths are unchanged, at every node", () => {
        const swapped = opts({ mode: "delta", swapped: true })
        for (let i = 0; i < NAMES.length; i++) {
            expect(getMeasure(graph, i, swapped)).toBe(
                getMeasure(graph, i, delta),
            )
        }
    })

    test("swapped Slower shows exactly the frames of unswapped Faster", () => {
        const swapped = opts({ mode: "regressions", swapped: true })
        for (let i = 0; i < NAMES.length; i++) {
            expect(getMeasure(graph, i, swapped)).toBe(
                getMeasure(graph, i, faster),
            )
        }
    })

    test("tooltip presents A and B as reinterpreted", () => {
        // Build expectations with toLocaleString so the test is not
        // sensitive to the machine's number-formatting locale.
        const loc = (v: number) => BigInt(v).toLocaleString()
        expect(nodeDetails(NODE.Mixed, graph, delta)).toBe(
            `Mixed A: ${loc(2000)}, B: ${loc(2500)}, Δ: +${loc(500)}`,
        )
        expect(nodeDetails(NODE.Mixed, graph, opts({ swapped: true }))).toBe(
            `Mixed A: ${loc(2500)}, B: ${loc(2000)}, Δ: ${loc(-500)}`,
        )
    })
})

describe("collapse chains", () => {
    test("no chain through a node with multiple changed children", () => {
        expect(getFullWidthChild(graph, NODE.Mixed, delta)).toBe(-1)
    })

    test("a child carrying the parent's full width forms a chain", () => {
        // In Slower rooted anywhere, IO's only changed-slower child is mmap,
        // and IO has no self-delta, so mmap renders at IO's full width.
        expect(getFullWidthChild(graph, NODE.IO, slower)).toBe(NODE.mmap)
        expect(getSameWidthChain(NODE.IO, graph, slower)).toEqual([NODE.mmap])
    })
})
