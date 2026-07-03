import { DiffGraph, StackGraph } from "./stackGraph"
import { colorFor, type ColorSettings } from "./color"

/**
 * How a DiffGraph is rendered.
 *
 * "delta": node width is |delta| = |b - a| of the node's inclusive value.
 * Subtrees whose changes cancel out get zero width and disappear; children
 * are scaled down proportionally where their absolute deltas overflow the
 * parent's net delta.
 *
 * "regressions" / "improvements": one-sided views whose widths are the
 * positive (negative) part of each node's net delta. Subtrees whose net
 * delta has the opposite sign vanish; drill into a subtree to reveal
 * counter-flows inside it (the render root always acts as a container).
 */
export type DiffRenderMode = "delta" | "regressions" | "improvements"

export interface DiffRenderOptions {
    mode: DiffRenderMode
    /**
     * Reinterpret A as B and B as A. Flips the sign of every delta, so
     * red/green swap and the regressions/improvements views trade places.
     */
    swapped: boolean
}

export const DEFAULT_DIFF_RENDER_OPTIONS: DiffRenderOptions = {
    mode: "delta",
    swapped: false,
}

const absBig = (v: bigint): bigint => (v < 0n ? -v : v)
const posBig = (v: bigint): bigint => (v > 0n ? v : 0n)

/** The node's inclusive delta, sign-flipped when A/B are swapped. */
const deltaOf = (
    graph: DiffGraph,
    i: number,
    options: DiffRenderOptions,
): bigint => {
    const d = graph.deltas[i] ?? 0n
    return options.swapped ? -d : d
}

/** The node's self-time delta, sign-flipped when A/B are swapped. */
const selfDeltaOf = (
    graph: DiffGraph,
    i: number,
    options: DiffRenderOptions,
): bigint => {
    const d = graph.selfDeltas[i] ?? 0n
    return options.swapped ? -d : d
}

/**
 * Returns the value determining a node's ideal rendered width, before any
 * within-parent scaling.
 *
 * For StackGraph: the node's sample count.
 * For DiffGraph: mode-dependent — see DiffRenderMode.
 */
export const getMeasure = (
    graph: StackGraph | DiffGraph,
    i: number,
    options: DiffRenderOptions,
): bigint => {
    if (graph instanceof StackGraph) return graph.values[i] ?? 0n
    switch (options.mode) {
        case "delta":
            return absBig(deltaOf(graph, i, options))
        case "regressions":
            return posBig(deltaOf(graph, i, options))
        case "improvements":
            return posBig(-deltaOf(graph, i, options))
    }
}

/** The portion of a diff node's measure attributable to its self time. */
const getSelfMeasure = (
    graph: DiffGraph,
    i: number,
    options: DiffRenderOptions,
): bigint => {
    const selfDelta = selfDeltaOf(graph, i, options)
    switch (options.mode) {
        case "delta":
            return absBig(selfDelta)
        case "regressions":
            return posBig(selfDelta)
        case "improvements":
            return posBig(-selfDelta)
    }
}

/**
 * Returns the denominator children are laid out against: children with
 * measure `m` get `parentWidth * m / denominator`. For DiffGraphs this can
 * exceed the parent's own measure (offsetting child deltas), in which case
 * children are proportionally scaled down to fit.
 */
const getLayoutDenominator = (
    graph: StackGraph | DiffGraph,
    nodeId: number,
    children: ArrayLike<number>,
    options: DiffRenderOptions,
): bigint => {
    let childSum = 0n
    for (let k = 0; k < children.length; k++) {
        childSum += getMeasure(graph, children[k]!, options)
    }
    if (graph instanceof StackGraph) {
        const m = getMeasure(graph, nodeId, options)
        return childSum > m ? childSum : m
    }
    return childSum + getSelfMeasure(graph, nodeId, options)
}

/**
 * Returns whether there is anything to draw below `rootNode`.
 *
 * Usually this means the root's own measure is positive, but in the net
 * signed views the render root acts as a container: subtrees that moved
 * against the root's own net direction must still render (e.g. the Faster
 * view under a root that got net slower), so the root counts as renderable
 * whenever any of its layout shares are positive.
 */
export const hasRenderableContent = (
    graph: StackGraph | DiffGraph,
    rootNode: number,
    options: DiffRenderOptions,
): boolean => {
    if (getMeasure(graph, rootNode, options) > 0n) return true
    if (graph instanceof DiffGraph && options.mode !== "delta") {
        return (
            getLayoutDenominator(
                graph,
                rootNode,
                graph.getChildren(rootNode),
                options,
            ) > 0n
        )
    }
    return false
}

/**
 * Diff node color: red where the node got slower, green where it got
 * faster, with saturation scaled by the relative change |delta| / (a + b).
 */
const diffColorFor = (
    graph: DiffGraph,
    nodeId: number,
    options: DiffRenderOptions,
): string => {
    const a = graph.valuesA[nodeId] ?? 0n
    const b = graph.valuesB[nodeId] ?? 0n
    const delta = deltaOf(graph, nodeId, options)
    const total = a + b
    if (total === 0n || delta === 0n) return "hsl(0, 0%, 55%)"
    // Relative change, scaled so a 50% shift is fully saturated.
    const ratio = Math.min(1, (2 * Math.abs(Number(delta))) / Number(total))
    const sat = Math.round(15 + 70 * ratio)
    return `hsl(${delta > 0n ? 4 : 140}, ${sat}%, 58%)`
}

// --- Constants ---

/** Nodes with pixel width below this threshold will not be rendered. */
export const CULLING_THRESHOLD_PX = 5

/** The number of consecutive nodes with the same width required to make a node collapsible. */
export const COLLAPSE_THRESHOLD = 2

/** The height of each node, in pixels. */
export const NODE_HEIGHT = 22

/** The horizontal width of the coordinate system. */
export const COORDINATE_WIDTH = 100_000

/** The space between the left edge of a node and its text label, in pixels. */
const NODE_TEXT_PADDING_LEFT = 5

const FONT_SIZE = 12
const FONT = `${FONT_SIZE}px sans-serif`

/**
 * Returns the single child that renders at exactly the parent's width, or -1
 * when there is none: exactly one child with a nonzero measure, and that
 * measure equals the full layout denominator (no self share, no other
 * changed siblings).
 */
export const getFullWidthChild = (
    graph: StackGraph | DiffGraph,
    nodeId: number,
    options: DiffRenderOptions,
): number => {
    const children = graph.getChildren(nodeId)
    let only = -1
    let onlyMeasure = 0n
    for (let k = 0; k < children.length; k++) {
        const childId = children[k]!
        const m = getMeasure(graph, childId, options)
        if (m > 0n) {
            if (only !== -1) return -1
            only = childId
            onlyMeasure = m
        }
    }
    if (only === -1) return -1
    const denom = getLayoutDenominator(graph, nodeId, children, options)
    return onlyMeasure === denom ? only : -1
}

export const getSameWidthChain = (
    nodeId: number,
    graph: StackGraph | DiffGraph,
    options: DiffRenderOptions,
): number[] => {
    const chain = []
    let currentId = nodeId
    while (true) {
        const childId = getFullWidthChild(graph, currentId, options)
        if (childId === -1) break
        chain.push(childId)
        currentId = childId
    }
    return chain
}

const signedBig = (v: bigint): string =>
    v > 0n ? `+${v.toLocaleString()}` : v.toLocaleString()

export const nodeDetails = (
    nodeId: number,
    graph: StackGraph | DiffGraph,
    options: DiffRenderOptions,
): string => {
    if (graph instanceof DiffGraph) {
        const rawA = graph.valuesA[nodeId]
        const rawB = graph.valuesB[nodeId]
        if (rawA == undefined || rawB == undefined)
            return "Malformed graph. Missing node data."
        // When swapped, present the graphs as the user is interpreting them.
        const a = options.swapped ? rawB : rawA
        const b = options.swapped ? rawA : rawB
        const delta = deltaOf(graph, nodeId, options)
        return (
            `${graph.getNodeName(nodeId)} A: ${a.toLocaleString()}, ` +
            `B: ${b.toLocaleString()}, Δ: ${signedBig(delta)}`
        )
    }
    const value = graph.values[nodeId]
    if (value == undefined) return "Malformed graph. Missing node data."
    return `${graph.getNodeName(nodeId)} ${BigInt.asIntN(64, value).toLocaleString()} samples`
}

/** A rendered node rectangle, used for mouse hit testing. */
export interface RenderedNode {
    nodeId: number
    /** Canvas-pixel x of the left edge. */
    x: number
    /** Canvas-pixel y of the top edge. */
    y: number
    /** Canvas-pixel width. */
    width: number
    /** Canvas-pixel height. */
    height: number
    /** The node's full name (for hover highlight matching). */
    name: string
    /** True if this entry is the collapse/expand toggle button, not the node body. */
    isCollapseToggle: boolean
}

/**
 * Draws the entire flamegraph onto a canvas and returns the list of rendered
 * node rectangles so callers can do mouse hit testing without an extra pass.
 *
 * Layout is computed top-down in float coordinate space: the root spans
 * COORDINATE_WIDTH and each child gets its parent's width times
 * measure / layoutDenominator. For StackGraphs this reproduces the classic
 * sample-proportional layout; for DiffGraphs widths are shares of change,
 * scaled down wherever offsetting child deltas overflow the parent.
 */
export function drawFlamegraph(
    ctx: CanvasRenderingContext2D,
    graph: StackGraph | DiffGraph,
    rootNode: number,
    viewLeft: number,
    viewRight: number,
    canvasWidth: number,
    canvasHeight: number,
    dpr: number,
    expandedNodes: Set<number>,
    colorSettings: ColorSettings,
    hoveredName: string | null,
    hoveredCollapseNodeId: number | null,
    searchQuery: string | undefined,
    diffRenderOptions: DiffRenderOptions,
): RenderedNode[] {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, canvasWidth, canvasHeight)

    if (!hasRenderableContent(graph, rootNode, diffRenderOptions)) return []

    const searchQueryLower = searchQuery?.toLowerCase()
    const zoomWidth = viewRight - viewLeft
    const renderedNodes: RenderedNode[] = []

    ctx.font = FONT
    ctx.textBaseline = "middle"
    ctx.textAlign = "left"

    type StackEntry = {
        nodeId: number
        /** Left edge in coordinate space (COORDINATE_WIDTH units). */
        xCoord: number
        /** Width in coordinate space. */
        widthCoord: number
        depth: number
        parentWidthCoord: number | null
    }

    const stack: StackEntry[] = [
        {
            nodeId: rootNode,
            xCoord: 0,
            widthCoord: COORDINATE_WIDTH,
            depth: 0,
            parentWidthCoord: null,
        },
    ]

    while (stack.length > 0) {
        const { nodeId, xCoord, widthCoord, depth, parentWidthCoord } =
            stack.pop()!

        const name = graph.getNodeName(nodeId)

        // Map from coordinate space to canvas pixels.
        const canvasX = ((xCoord - viewLeft) / zoomWidth) * canvasWidth
        const canvasW = (widthCoord / zoomWidth) * canvasWidth

        if (canvasW < CULLING_THRESHOLD_PX) continue

        // Viewport culling: skip nodes entirely off-screen horizontally.
        // Children are contained within the parent, so the entire subtree
        // can be skipped.
        if (canvasX + canvasW <= 0 || canvasX >= canvasWidth) continue

        const canvasY = canvasHeight - (depth + 1) * NODE_HEIGHT

        // Collapse/expand: only compute the chain when there is a chance the
        // node is collapsible (a node filling its parent exactly is already
        // part of the parent's chain).
        let sameWidthChain: number[] | undefined
        let isCollapsible = false
        let isCollapsed = false

        if (widthCoord !== parentWidthCoord) {
            sameWidthChain = getSameWidthChain(nodeId, graph, diffRenderOptions)
            isCollapsible = sameWidthChain.length >= COLLAPSE_THRESHOLD
            isCollapsed = isCollapsible && !expandedNodes.has(nodeId)
        }

        // Node body.
        ctx.fillStyle =
            graph instanceof DiffGraph
                ? diffColorFor(graph, nodeId, diffRenderOptions)
                : colorFor(name, colorSettings)
        ctx.fillRect(canvasX, canvasY, canvasW, NODE_HEIGHT)

        const showCollapseButton = isCollapsible && canvasW >= NODE_HEIGHT * 2

        // The body area excludes the collapse button so overlays never dim it.
        const bodyX = showCollapseButton ? canvasX + NODE_HEIGHT : canvasX
        const bodyW = showCollapseButton ? canvasW - NODE_HEIGHT : canvasW

        const nodeMatchesSearch =
            !searchQueryLower || name.toLowerCase().includes(searchQueryLower)
        // True when the chain contains a search match — the +/− button stays undimmed
        // regardless of whether the chain is currently expanded or collapsed.
        const chainMatchesSearch =
            !!searchQueryLower &&
            sameWidthChain != null &&
            sameWidthChain.some((id) =>
                graph.getNodeName(id).toLowerCase().includes(searchQueryLower),
            )
        // Dim the node body if nothing relevant (node or hidden chain) matches.
        const isDimmed =
            !!searchQueryLower && !nodeMatchesSearch && !chainMatchesSearch
        // Dim the button when there are no hidden matches to reveal.
        const btnDimmed =
            showCollapseButton && !!searchQueryLower && !chainMatchesSearch

        if (isDimmed) {
            ctx.fillStyle = "rgba(0,0,0,0.5)"
            if (btnDimmed) {
                // Entire row dimmed: one rect avoids a sub-pixel gap at the
                // button/body boundary.
                ctx.fillRect(canvasX, canvasY, canvasW, NODE_HEIGHT)
            } else {
                ctx.fillRect(bodyX, canvasY, bodyW, NODE_HEIGHT)
            }
        } else if (btnDimmed) {
            // Node matches but the button still needs dimming.
            ctx.fillStyle = "rgba(0,0,0,0.5)"
            ctx.fillRect(canvasX, canvasY, NODE_HEIGHT, NODE_HEIGHT)
        }

        // Hover overlay for the node body, excluding the button area.
        if (name === hoveredName) {
            ctx.fillStyle = "rgba(0,0,0,0.25)"
            ctx.fillRect(bodyX, canvasY, bodyW, NODE_HEIGHT)
        }

        // 1px border at the bottom of each row.
        ctx.fillStyle = "rgba(0,0,0,0.15)"
        ctx.fillRect(canvasX, canvasY + NODE_HEIGHT - 1, canvasW, 1)

        renderedNodes.push({
            nodeId,
            x: canvasX,
            y: canvasY,
            width: canvasW,
            height: NODE_HEIGHT,
            name,
            isCollapseToggle: false,
        })

        if (showCollapseButton) {
            const btnX = canvasX
            const btnY = canvasY

            if (hoveredCollapseNodeId === nodeId) {
                ctx.fillStyle = "rgba(0,0,0,0.25)"
                ctx.fillRect(btnX, btnY, NODE_HEIGHT, NODE_HEIGHT)
            }

            ctx.fillStyle = "black"
            ctx.textAlign = "center"
            ctx.fillText(
                isCollapsed ? "+" : "−",
                btnX + NODE_HEIGHT / 2,
                btnY + NODE_HEIGHT / 2,
            )
            ctx.textAlign = "left"

            renderedNodes.push({
                nodeId,
                x: btnX,
                y: btnY,
                width: NODE_HEIGHT,
                height: NODE_HEIGHT,
                name,
                isCollapseToggle: true,
            })
        }

        // Text label — only render when the available pixel width can fit at
        // least a couple of characters.
        const textOffsetX = showCollapseButton
            ? NODE_HEIGHT
            : NODE_TEXT_PADDING_LEFT
        const availableTextWidth = canvasW - textOffsetX

        if (availableTextWidth > FONT_SIZE) {
            const displayName = graph.getDisplayName(nodeId)
            const textWidth = ctx.measureText(displayName).width
            const textX = canvasX + textOffsetX
            ctx.fillStyle = "black"
            if (textWidth <= availableTextWidth) {
                ctx.fillText(displayName, textX, canvasY + NODE_HEIGHT / 2)
            } else {
                ctx.save()
                ctx.beginPath()
                ctx.rect(textX, canvasY, availableTextWidth, NODE_HEIGHT)
                ctx.clip()
                ctx.fillText(displayName, textX, canvasY + NODE_HEIGHT / 2)
                ctx.restore()
            }
        }

        // Push children onto the stack.
        const children = isCollapsed
            ? graph.getChildren(sameWidthChain![sameWidthChain!.length - 1]!)
            : graph.getChildren(nodeId)

        if (children.length === 0) continue

        const denom = getLayoutDenominator(
            graph,
            // Children of the chain tail are laid out against the tail's own
            // denominator so their widths are unaffected by the collapse.
            isCollapsed ? sameWidthChain![sameWidthChain!.length - 1]! : nodeId,
            children,
            diffRenderOptions,
        )
        if (denom <= 0n) continue
        const denomNum = Number(denom)

        // Children keep the merged graph's order so frames stay in the same
        // relative position across render modes and plain flamegraphs.
        let childX = xCoord
        for (let k = 0; k < children.length; k++) {
            const childId = children[k]!
            const m = getMeasure(graph, childId, diffRenderOptions)
            if (m <= 0n) continue
            const childW = widthCoord * (Number(m) / denomNum)

            // Children are laid out left-to-right, so once the
            // left edge is past the viewport we can stop.
            if (childX >= viewRight) break

            // Skip children entirely before the viewport.
            if (
                childX + childW > viewLeft &&
                (childW / zoomWidth) * canvasWidth >= CULLING_THRESHOLD_PX
            ) {
                stack.push({
                    nodeId: childId,
                    xCoord: childX,
                    widthCoord: childW,
                    depth: depth + 1,
                    parentWidthCoord: widthCoord,
                })
            }
            childX += childW
        }
    }

    return renderedNodes
}
