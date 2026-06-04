// MCP server for flamegraph profiles — compiled at build time and embedded in the flamegraph HTML.
// Downloaded file has a shebang and __WASM_BASE64__ const prepended, and embedded stack data
// appended as comments after a sentinel line followed by name/data line pairs.

import { createReadStream } from "node:fs"
import { createInterface } from "node:readline"
import {
    initSync,
    StacksParser,
    WasmStackGraph,
    wasm_icicle_graph,
    wasm_merge_children,
} from "@flamegraph-wasm"
import { Graph, nodeCount } from "./stackGraph.ts"

// __WASM_BASE64__ is injected at build time by the Vite plugin via esbuild define.
declare const __WASM_BASE64__: string

const binaryStr = atob(__WASM_BASE64__)
const wasmBuf = new Uint8Array(binaryStr.length)
for (let i = 0; i < binaryStr.length; i++) {
    wasmBuf[i] = binaryStr.charCodeAt(i)
}
initSync({ module: wasmBuf.buffer })

function wasmGraphToGraph(wg: WasmStackGraph): Graph {
    const raw = {
        childrenOffsets: wg.children_offsets(),
        childrenData: wg.children_data(),
        namesData: wg.names_data(),
        namesOffsets: wg.names_offsets(),
        displayNamesData: wg.display_names_data(),
        displayNamesOffsets: wg.display_names_offsets(),
        values: wg.values(),
    }
    wg.free()
    return new Graph(raw)
}

const NEWLINE_BYTE = 0x0a
const NEWLINE = Buffer.from([NEWLINE_BYTE])
const SENTINEL = Buffer.concat([NEWLINE, Buffer.from("// EMBEDDED_DATA_START"), NEWLINE])
const NAME_PREFIX = "// STACKS_NAME "
const DATA_PREFIX = Buffer.from("// STACKS_DATA ")

/** Streaming base64 decoder that operates on Buffer chunks. */
class Base64Decoder {
    private remainder = Buffer.alloc(0)

    decode(chunk: Buffer): Buffer {
        const input =
            this.remainder.length > 0
                ? Buffer.concat([this.remainder, chunk])
                : chunk
        const usable = input.length - (input.length % 4)
        this.remainder =
            usable < input.length
                ? Buffer.from(input.subarray(usable))
                : Buffer.alloc(0)
        if (usable === 0) {
            return Buffer.alloc(0)
        }
        return Buffer.from(
            input.subarray(0, usable).toString("ascii"),
            "base64",
        )
    }

    flush(): Buffer {
        if (this.remainder.length === 0) {
            return Buffer.alloc(0)
        }
        const result = Buffer.from(
            this.remainder.toString("ascii"),
            "base64",
        )
        this.remainder = Buffer.alloc(0)
        return result
    }
}

/** Decode a base64 stream → decompress (deflate-raw) → parse into a Graph. */
async function parseGraph(
    base64Source: ReadableStream<Buffer>,
): Promise<Graph> {
    const decoder = new Base64Decoder()
    const ds = new DecompressionStream("deflate-raw")
    const writer = ds.writable.getWriter()

    // Feed base64 → decode → write to decompressor (runs concurrently with
    // the parse loop below via microtask interleaving).
    const feedDone = (async () => {
        const reader = base64Source.getReader()
        while (true) {
            const { done, value } = await reader.read()
            if (value) {
                const decoded = decoder.decode(value)
                if (decoded.length > 0) {
                    await writer.write(decoded as Uint8Array<ArrayBuffer>)
                }
            }
            if (done) {
                break
            }
        }
        const flushed = decoder.flush()
        if (flushed.length > 0) {
            await writer.write(flushed as Uint8Array<ArrayBuffer>)
        }
        await writer.close()
    })()

    const parser = new StacksParser("root")
    const reader = ds.readable.getReader()
    while (true) {
        const { done, value } = await reader.read()
        if (value) {
            parser.feed(value)
        }
        if (done) {
            break
        }
    }

    await feedDone
    return wasmGraphToGraph(parser.finish())
}

/**
 * Stream through this script file, find the embedded data sentinel, and
 * yield parsed graphs. Base64 data flows through a streaming pipeline
 * (base64 decode → decompress → parse) without being fully buffered.
 */
async function* readEmbeddedStacks(): AsyncGenerator<{
    name: string
    graph: Graph
}> {
    const scriptPath = process.argv[1]
    if (!scriptPath) {
        throw new Error("Cannot determine script path from process.argv[1]")
    }

    // State machine phases.
    const SCANNING = 0 // scanning for sentinel
    const READING_NAME = 1 // reading a STACKS_NAME line
    const MATCHING_PREFIX = 2 // matching the STACKS_DATA prefix
    const STREAMING_DATA = 3 // streaming base64 bytes until newline

    let state = SCANNING
    let sentinelMatched = 0
    let lineChunks: Buffer[] = []
    let prefixMatched = 0
    let currentName = ""

    // In STREAMING_DATA state, base64 chunks are pushed into this controller.
    // parseGraph reads from the corresponding ReadableStream.
    let dataController: ReadableStreamDefaultController<Buffer> | null = null
    let graphPromise: Promise<Graph> | null = null

    for await (const rawChunk of createReadStream(scriptPath)) {
        const chunk = rawChunk as Buffer
        let pos = 0

        while (pos < chunk.length) {
            if (state === SCANNING) {
                while (pos < chunk.length) {
                    if (chunk[pos] === SENTINEL[sentinelMatched]) {
                        sentinelMatched++
                        pos++
                        if (sentinelMatched === SENTINEL.length) {
                            state = READING_NAME
                            break
                        }
                    } else if (sentinelMatched > 0) {
                        sentinelMatched = 0
                        // Re-check current byte as potential match start.
                        if (chunk[pos] === SENTINEL[0]) {
                            sentinelMatched = 1
                        }
                        pos++
                    } else {
                        pos++
                    }
                }
            } else if (state === READING_NAME) {
                const nlIdx = chunk.indexOf(NEWLINE_BYTE, pos)
                if (nlIdx === -1) {
                    lineChunks.push(Buffer.from(chunk.subarray(pos)))
                    pos = chunk.length
                } else {
                    lineChunks.push(chunk.subarray(pos, nlIdx))
                    const line = Buffer.concat(lineChunks).toString("utf-8")
                    lineChunks = []
                    pos = nlIdx + 1
                    if (!line.startsWith(NAME_PREFIX)) {
                        throw new Error(`Expected "${NAME_PREFIX}" but got: ${JSON.stringify(line.slice(0, 100))}`)
                    }
                    currentName = line.slice(NAME_PREFIX.length)
                    state = MATCHING_PREFIX
                    prefixMatched = 0
                }
            } else if (state === MATCHING_PREFIX) {
                while (
                    pos < chunk.length &&
                    prefixMatched < DATA_PREFIX.length
                ) {
                    if (chunk[pos] !== DATA_PREFIX[prefixMatched]) {
                        throw new Error(`Expected "${DATA_PREFIX.toString()}" after STACKS_NAME`)
                    }
                    prefixMatched++
                    pos++
                }
                if (prefixMatched === DATA_PREFIX.length) {
                    // Start the streaming pipeline for this entry.
                    const stream = new ReadableStream<Buffer>({
                        start(c) {
                            dataController = c
                        },
                    })
                    graphPromise = parseGraph(stream)
                    state = STREAMING_DATA
                }
            } else {
                // STREAMING_DATA: push base64 bytes until newline.
                const nlIdx = chunk.indexOf(NEWLINE_BYTE, pos)
                if (nlIdx === -1) {
                    if (pos < chunk.length) {
                        dataController!.enqueue(
                            Buffer.from(chunk.subarray(pos)),
                        )
                    }
                    pos = chunk.length
                } else {
                    if (nlIdx > pos) {
                        dataController!.enqueue(
                            Buffer.from(chunk.subarray(pos, nlIdx)),
                        )
                    }
                    dataController!.close()
                    yield {
                        name: currentName,
                        graph: await graphPromise!,
                    }
                    dataController = null
                    graphPromise = null
                    pos = nlIdx + 1
                    state = READING_NAME
                }
            }
        }
    }

    // Handle EOF while streaming data (no trailing newline).
    if (
        state === STREAMING_DATA &&
        (dataController as ReadableStreamDefaultController<Buffer> | null) &&
        graphPromise
    ) {
        dataController!.close()
        yield { name: currentName, graph: await graphPromise }
    }
}

function formatSamples(n: bigint, total: bigint): string {
    const pct = total > 0n ? (Number(n) / Number(total) * 100).toFixed(1) : "0.0"
    return `${n.toLocaleString()} samples (${pct}%)`
}

interface GraphEntry {
    id: number
    name: string
    graph: Graph
}

const graphMap = new Map<number, GraphEntry>()
let nextGraphId = 0

function registerGraph(name: string, graph: Graph): GraphEntry {
    const id = nextGraphId++
    const entry = { id, name, graph }
    graphMap.set(id, entry)
    return entry
}

function resolveGraph(graphId: number): GraphEntry | string {
    if (graphMap.size === 0) {
        return "No profiles loaded."
    }
    const found = graphMap.get(graphId)
    if (!found) {
        return `Graph ${graphId} not found. Available IDs: ${[...graphMap.keys()].join(", ")}`
    }
    return found
}

/** Build a parent map: result[i] = parent node ID, or -1 for root. */
function buildParentMap(graph: Graph): Int32Array {
    const n = graph.nodeCount
    const parent = new Int32Array(n).fill(-1)
    for (let i = 0; i < n; i++) {
        for (const child of graph.getChildren(i)) {
            parent[child] = i
        }
    }
    return parent
}

/** Resolve a node ID from either a direct node_id or a regex pattern (picks highest-sample match). */
function resolveNodeByIdOrPattern(graph: Graph, name: string, nodeId?: number, pattern?: string): number | string {
    if (nodeId != null) {
        if (nodeId < 0 || nodeId >= graph.nodeCount) {
            return `Node ID ${nodeId} out of range (0–${graph.nodeCount - 1}).`
        }
        return nodeId
    }
    if (pattern != null) {
        let regex: RegExp
        try {
            regex = new RegExp(pattern, "i")
        } catch (e: any) {
            return `Invalid regex: ${e?.message}`
        }
        let bestId = -1
        let bestValue = -1n
        for (let i = 1; i < graph.nodeCount; i++) {
            if (regex.test(graph.getNodeName(i)) && (graph.values[i] ?? 0n) > bestValue) {
                bestId = i
                bestValue = graph.values[i] ?? 0n
            }
        }
        if (bestId === -1) {
            return `No nodes matching /${pattern}/i in "${name}".`
        }
        return bestId
    }
    return "Either node_id or pattern must be provided."
}

/** A caller set: maps real node IDs to the weight attributed from below. */
type CallerSet = Map<number, bigint>

/**
 * Abstract weighted tree that both forward (subtree) and reverse (caller) views implement.
 * The budget-limited tree builder and box-drawing renderer operate on this interface.
 */
interface WeightedTree<N> {
    displayName(node: N): string
    weight(node: N): bigint
    selfWeight(node: N): bigint
    childCount(node: N): number
    /** Returns children sorted by weight descending. */
    sortedChildren(node: N): N[]
    /** Optional node ID for display (e.g. forward graph has IDs, caller graph doesn't). */
    nodeId?(node: N): number | undefined
}

interface SummaryNode {
    nodeId?: number
    displayName: string
    value: bigint
    selfValue: bigint
    totalChildren: number
    children: SummaryNode[]
}

/** Budget-limited tree builder. Returns actual lines used so unused budget flows to siblings. */
function buildTree<N>(view: WeightedTree<N>, root: N, budget: number): SummaryNode {
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
function renderTree(node: SummaryNode, total: bigint, childLabel: string, ownPrefix: string, childPrefix: string, lines: string[]): void {
    const selfStr = node.selfValue > 0n ? `, self: ${formatSamples(node.selfValue, total)}` : ""
    const childStr = node.totalChildren > 0 ? `, ${childLabel}: ${node.totalChildren}` : ""
    const idStr = node.nodeId != null ? `[${node.nodeId}] ` : ""
    lines.push(`${ownPrefix}${idStr}${node.displayName}, ${formatSamples(node.value, total)}${selfStr}${childStr}`)
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

function toolListGraphs(): string {
    if (graphMap.size === 0) {
        return "No profiles loaded."
    }
    const lines = [...graphMap.values()].map(({ id, name, graph }) => {
        const total = graph.values[0] ?? 0n
        return `  [${id}] ${name} — ${graph.nodeCount.toLocaleString()} nodes, ${total.toLocaleString()} total samples, root node: 0`
    })
    return `${graphMap.size} profile(s) loaded:\n${lines.join("\n")}`
}

function toolSearchByName(graphId: number, pattern: string, limit = 10, offset = 0, rootNodeId = 0): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    if (rootNodeId < 0 || rootNodeId >= graph.nodeCount) {
        return `Node ID ${rootNodeId} out of range (0–${graph.nodeCount - 1}).`
    }

    let regex: RegExp
    try {
        regex = new RegExp(pattern, "i")
    } catch (e: any) {
        return `Invalid regex: ${e?.message}`
    }

    const total = graph.values[0] ?? 0n

    // Collect nodes to search: if rooted at a subtree, BFS from rootNodeId.
    const searchNodes: number[] = []
    if (rootNodeId === 0) {
        for (let i = 1; i < graph.nodeCount; i++) {
            searchNodes.push(i)
        }
    } else {
        const queue = [rootNodeId]
        for (let head = 0; head < queue.length; head++) {
            const nid = queue[head]!
            searchNodes.push(nid)
            for (const child of graph.getChildren(nid)) {
                queue.push(child)
            }
        }
    }

    const matches: Array<{ nodeId: number; value: bigint; selfValue: bigint }> = []
    for (const i of searchNodes) {
        if (regex.test(graph.getNodeName(i))) {
            const value = graph.values[i] ?? 0n
            let selfValue = value
            for (const child of graph.getChildren(i)) {
                selfValue -= graph.values[child] ?? 0n
            }
            matches.push({ nodeId: i, value, selfValue })
        }
    }

    if (matches.length === 0) {
        return `No nodes matching /${pattern}/i in "${name}"${rootNodeId !== 0 ? ` under [${rootNodeId}]` : ""}.`
    }

    matches.sort((a, b) => Number(b.value - a.value))
    const matchTotal = matches.reduce((sum, { value }) => sum + value, 0n)
    const shown = matches.slice(offset, offset + limit)
    const parentMap = buildParentMap(graph)
    const lines = shown.map(({ nodeId, value, selfValue }) => {
        const parentId = parentMap[nodeId] ?? -1
        const parentStr = parentId === -1 ? "root" : `[${parentId}] ${graph.getDisplayName(parentId)}`
        const selfStr = selfValue > 0n ? `, self: ${formatSamples(selfValue, total)}` : ""
        return `  [${nodeId}] ${graph.getDisplayName(nodeId)}, ${formatSamples(value, total)}${selfStr}, parent: ${parentStr}`
    })
    const remaining = matches.length - offset - shown.length
    const suffix = remaining > 0 ? `\n  (${remaining} more not shown)` : ""
    const scopeStr = rootNodeId !== 0 ? ` under [${rootNodeId}]` : ""
    return `${matches.length} node(s) matching /${pattern}/i in "${name}"${scopeStr} — combined: ${formatSamples(matchTotal, total)} (showing ${shown.length} from offset ${offset}):\n${lines.join("\n")}${suffix}`
}

function toolNodeSummary(graphId: number, nodeId: number, parentDepth = 10): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    if (nodeId < 0 || nodeId >= graph.nodeCount) {
        return `Node ID ${nodeId} out of range (0–${graph.nodeCount - 1}).`
    }

    const total = graph.values[0] ?? 0n
    const nodeValue = graph.values[nodeId] ?? 0n
    const children = graph.getChildren(nodeId)

    let selfValue = nodeValue
    for (const child of children) {
        selfValue -= graph.values[child] ?? 0n
    }

    const parentMap = buildParentMap(graph)
    const chain: Array<{ nodeId: number; name: string }> = []
    let cur = parentMap[nodeId] ?? -1
    while (cur !== -1 && (parentDepth === -1 || chain.length < parentDepth)) {
        chain.push({ nodeId: cur, name: graph.getDisplayName(cur) })
        cur = parentMap[cur] ?? -1
    }
    const truncated = cur !== -1
    const parentChainStr = chain.length === 0
        ? "  (root)"
        : chain.map(p => `  [${p.nodeId}] ${p.name}`).join("\n") + (truncated ? "\n  (truncated)" : "")

    return [
        `Node [${nodeId}] in "${name}":`,
        `  Name: ${graph.getDisplayName(nodeId)}`,
        `  Total samples: ${formatSamples(nodeValue, total)}`,
        `  Self samples:  ${formatSamples(selfValue, total)}`,
        `  Children: ${children.length}`,
        `  Parent chain (node → root):`,
        parentChainStr,
    ].join("\n")
}

function toolChildrenOfNode(graphId: number, nodeId: number, limit = 10): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    if (nodeId < 0 || nodeId >= graph.nodeCount) {
        return `Node ID ${nodeId} out of range (0–${graph.nodeCount - 1}).`
    }

    const parentValue = graph.values[nodeId] ?? 0n
    const children = Array.from(graph.getChildren(nodeId))
    children.sort((a, b) => Number((graph.values[b] ?? 0n) - (graph.values[a] ?? 0n)))

    if (children.length === 0) {
        return `Node [${nodeId}] "${graph.getDisplayName(nodeId)}" has no children.`
    }

    const shown = limit === -1 ? children : children.slice(0, limit)
    const lines = shown.map(childId => {
        const childValue = graph.values[childId] ?? 0n
        let childSelf = childValue
        for (const grandchild of graph.getChildren(childId)) {
            childSelf -= graph.values[grandchild] ?? 0n
        }
        return `  [${childId}] ${graph.getDisplayName(childId)}  total: ${formatSamples(childValue, parentValue)}  self: ${formatSamples(childSelf, parentValue)}`
    })
    const suffix = limit !== -1 && children.length > limit
        ? `\n  (${children.length - limit} more not shown)`
        : ""
    return `Children of [${nodeId}] "${graph.getDisplayName(nodeId)}" in "${name}" (${shown.length}/${children.length}):\n${lines.join("\n")}${suffix}`
}

function toolAggregateByName(graphId: number, nodeId?: number, pattern?: string): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    const resolvedNodeId = resolveNodeByIdOrPattern(graph, name, nodeId, pattern)
    if (typeof resolvedNodeId === "string") {
        return resolvedNodeId
    }

    const nodeName = graph.getNodeName(resolvedNodeId)
    const raw = graph.toRaw()
    const wg = wasm_merge_children(
        raw.childrenOffsets,
        raw.childrenData,
        raw.namesData,
        raw.namesOffsets,
        raw.values,
        nodeName,
    )
    const newGraph = wasmGraphToGraph(wg)
    const entry = registerGraph(`${name} [merged: ${graph.getDisplayName(resolvedNodeId)}]`, newGraph)

    // Sum inclusive values of all matching nodes in the source graph.
    // If this exceeds the merged root, the difference represents samples that
    // passed through two or more matching nodes in the same call stack (nested
    // calls / recursion). Those samples are counted once per occurrence in the
    // sum but only once in the merged root, which is the authoritative figure.
    let sourceSum = 0n
    for (let i = 0; i < graph.nodeCount; i++) {
        if (graph.getNodeName(i) === nodeName) {
            sourceSum += graph.values[i] ?? 0n
        }
    }
    const mergedRoot = newGraph.values[0] ?? 0n
    const diff = sourceSum - mergedRoot
    const discrepancyNote = diff > 0n
        ? ` Note: individual node inclusive values sum to ${sourceSum.toLocaleString()} but merged root is ${mergedRoot.toLocaleString()} — the ${diff.toLocaleString()}-sample difference represents stacks where this function appears more than once (nested calls / recursion), counted once in the merged graph.`
        : ""

    return `Created merged graph [${entry.id}] "${entry.name}" with ${newGraph.nodeCount.toLocaleString()} nodes, root node: 0.${discrepancyNote}`
}

function toolIcicleGraph(graphId: number, rootNodeId?: number, pattern?: string): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    const resolvedRootId = resolveNodeByIdOrPattern(graph, name, rootNodeId, pattern)
    if (typeof resolvedRootId === "string") {
        return resolvedRootId
    }

    const raw = graph.toRaw()
    const wg = wasm_icicle_graph(
        raw.childrenOffsets,
        raw.childrenData,
        raw.namesData,
        raw.namesOffsets,
        raw.values,
        resolvedRootId,
        nodeCount(raw),
    )
    const newGraph = wasmGraphToGraph(wg)
    const entry = registerGraph(`${name} [icicle: ${graph.getDisplayName(resolvedRootId)}]`, newGraph)
    return `Created icicle graph [${entry.id}] "${entry.name}" with ${newGraph.nodeCount.toLocaleString()} nodes, root node: 0.`
}

function toolHotPath(graphId: number, nodeId: number, depthLimit: number | null = null): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    if (nodeId < 0 || nodeId >= graph.nodeCount) {
        return `Node ID ${nodeId} out of range (0–${graph.nodeCount - 1}).`
    }

    const total = graph.values[0] ?? 0n
    const path: Array<{ nodeId: number; value: bigint }> = []
    let cur = nodeId
    // Use while(true) so we can distinguish leaf-exit from depth-limit-exit.
    while (true) {
        path.push({ nodeId: cur, value: graph.values[cur] ?? 0n })
        const children = Array.from(graph.getChildren(cur))
        if (children.length === 0) {
            break // leaf
        }
        cur = children.reduce((best, c) =>
            (graph.values[c] ?? 0n) > (graph.values[best] ?? 0n) ? c : best
        )
        if (depthLimit !== null && path.length >= depthLimit) {
            break // depth limit
        }
    }

    // depthLimitHit is true only if we stopped because of the limit, not a leaf.
    // After a depth-limit break, cur is the next unvisited node (already one beyond path).
    const depthLimitHit = depthLimit !== null && path.length >= depthLimit
        && graph.getChildren(path[path.length - 1]!.nodeId).length > 0

    let depthNote = ""
    if (depthLimitHit) {
        // cur is the (path.length+1)th node; count from there to find the leaf.
        let trueDepth = path.length + 1
        while (true) {
            const children = Array.from(graph.getChildren(cur))
            if (children.length === 0) {
                break
            }
            cur = children.reduce((best, c) =>
                (graph.values[c] ?? 0n) > (graph.values[best] ?? 0n) ? c : best
            )
            trueDepth++
        }
        depthNote = ` (truncated at depth ${depthLimit}; full hot path: ${trueDepth} nodes)`
    }

    // Collapse consecutive frames with the same inclusive value (pass-through wrappers).
    const lines: string[] = []
    let i = 0
    while (i < path.length) {
        const { nodeId: nid, value } = path[i]!
        let self = value
        for (const child of graph.getChildren(nid)) {
            self -= graph.values[child] ?? 0n
        }
        lines.push(`  [${nid}] ${graph.getDisplayName(nid)}  total: ${formatSamples(value, total)}  self: ${formatSamples(self, total)}`)

        // Count how many subsequent frames have the same value.
        let j = i + 1
        while (j < path.length && path[j]!.value === value) j++
        const skipped = j - i - 1
        if (skipped > 0) {
            const last = path[j - 1]!
            let lastSelf = last.value
            for (const child of graph.getChildren(last.nodeId)) {
                lastSelf -= graph.values[child] ?? 0n
            }
            lines.push(`  ... ${skipped} frames with same total ...`)
            lines.push(`  [${last.nodeId}] ${graph.getDisplayName(last.nodeId)}  total: ${formatSamples(last.value, total)}  self: ${formatSamples(lastSelf, total)}`)
        }
        i = j
    }
    return `Hot path from [${nodeId}] in "${name}" (${path.length} nodes, ${lines.length} lines shown${depthNote}):\n${lines.join("\n")}`
}

function toolRawName(graphId: number, nodeId: number): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { graph } = resolved

    if (nodeId < 0 || nodeId >= graph.nodeCount) {
        return `Node ID ${nodeId} out of range (0–${graph.nodeCount - 1}).`
    }

    return graph.getNodeName(nodeId)
}

function toolCallers(graphId: number, nodeId?: number, pattern?: string, maxLines = 40): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    const resolvedNodeId = resolveNodeByIdOrPattern(graph, name, nodeId, pattern)
    if (typeof resolvedNodeId === "string") {
        return resolvedNodeId
    }

    const targetName = graph.getNodeName(resolvedNodeId)
    const parentMap = buildParentMap(graph)

    // Find all instances of this function.
    const instances: number[] = []
    for (let i = 0; i < graph.nodeCount; i++) {
        if (graph.getNodeName(i) === targetName) instances.push(i)
    }
    if (instances.length === 0) {
        return `No instances of "${graph.getDisplayName(resolvedNodeId)}" found.`
    }

    // Build a CallerSet for the root: each instance maps to its own inclusive value.
    const rootSet = new Map<number, bigint>()
    for (const nid of instances) {
        rootSet.set(nid, (rootSet.get(nid) ?? 0n) + (graph.values[nid] ?? 0n))
    }

    const callerView: WeightedTree<CallerSet> = {
        displayName(cs) { return graph.getDisplayName(cs.keys().next().value!) },
        weight(cs) { let s = 0n; for (const v of cs.values()) s += v; return s },
        selfWeight(cs) {
            let parentWeight = 0n
            for (const [nid, w] of cs) {
                if ((parentMap[nid] ?? -1) !== -1) parentWeight += w
            }
            return this.weight(cs) - parentWeight
        },
        childCount(cs) {
            const names = new Set<string>()
            for (const nid of cs.keys()) {
                const pid = parentMap[nid] ?? -1
                if (pid !== -1) names.add(graph.getNodeName(pid))
            }
            return names.size
        },
        sortedChildren(cs) {
            const groups = new Map<string, CallerSet>()
            for (const [nid, w] of cs) {
                const pid = parentMap[nid] ?? -1
                if (pid === -1) continue
                const pname = graph.getNodeName(pid)
                let g = groups.get(pname)
                if (!g) { g = new Map(); groups.set(pname, g) }
                g.set(pid, (g.get(pid) ?? 0n) + w)
            }
            return [...groups.values()].sort((a, b) => {
                let wa = 0n, wb = 0n
                for (const v of a.values()) wa += v
                for (const v of b.values()) wb += v
                return Number(wb - wa)
            })
        },
    }

    const total = graph.values[0] ?? 0n
    const tree = buildTree(callerView, rootSet, maxLines)
    const lines: string[] = []
    renderTree(tree, total, "callers", "", "", lines)
    return `Callers of "${graph.getDisplayName(resolvedNodeId)}" in "${name}" (${instances.length.toLocaleString()} instances, ${lines.length} lines):\n${lines.join("\n")}`
}

function toolDeleteGraph(graphId: number): string {
    if (!graphMap.has(graphId)) {
        return `Graph ${graphId} not found. Available IDs: ${[...graphMap.keys()].join(", ")}`
    }
    const { name } = graphMap.get(graphId)!
    graphMap.delete(graphId)
    return `Deleted graph ${graphId} "${name}". ${graphMap.size} graph(s) remaining.`
}

function toolSubtreeSummary(graphId: number, nodeId: number, maxLines = 40): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    if (nodeId < 0 || nodeId >= graph.nodeCount) {
        return `Node ID ${nodeId} out of range (0–${graph.nodeCount - 1}).`
    }

    const forwardView: WeightedTree<number> = {
        displayName(n) { return graph.getDisplayName(n) },
        weight(n) { return graph.values[n] ?? 0n },
        selfWeight(n) {
            let self = graph.values[n] ?? 0n
            for (const c of graph.getChildren(n)) self -= graph.values[c] ?? 0n
            return self
        },
        childCount(n) { return graph.getChildren(n).length },
        sortedChildren(n) {
            return Array.from(graph.getChildren(n))
                .sort((a, b) => Number((graph.values[b] ?? 0n) - (graph.values[a] ?? 0n)))
        },
        nodeId(n) { return n },
    }

    const total = graph.values[0] ?? 0n
    const tree = buildTree(forwardView, nodeId, maxLines)
    const lines: string[] = []
    renderTree(tree, total, "children", "", "", lines)
    return `Subtree summary of [${nodeId}] in "${name}" (${lines.length} lines):\n${lines.join("\n")}`
}

const TOOLS = [
    {
        name: "list_graphs",
        description: "List all loaded flamegraph profiles with their IDs, node counts, and total sample counts.",
        inputSchema: { type: "object", properties: {}, required: [] },
    },
    {
        name: "search_by_name",
        description: "Search for nodes whose names match a regex pattern (case-insensitive). Returns matching node IDs sorted by sample count descending, with total and self samples.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID from list_graphs" },
                pattern: { type: "string", description: "Case-insensitive regex pattern to match against node names" },
                limit: { type: "number", description: "Maximum number of results (default 10)" },
                offset: { type: "number", description: "Number of results to skip for pagination (default 0)" },
                node_id: { type: "number", description: "Restrict search to the subtree rooted at this node (default: 0, the whole graph)" },
            },
            required: ["graph_id", "pattern"],
        },
    },
    {
        name: "node_details",
        description: "Get a summary of a node: name, self samples, total samples, number of children, and parent chain to root.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID from list_graphs" },
                node_id: { type: "number", description: "Node ID" },
                parent_depth: { type: "number", description: "How many levels of parent chain to show (default 10, -1 for unlimited)" },
            },
            required: ["graph_id", "node_id"],
        },
    },
    {
        name: "children_of_node",
        description: "List children of a node sorted by sample count descending.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID from list_graphs" },
                node_id: { type: "number", description: "Node ID" },
                limit: { type: "number", description: "Maximum number of children to return (default 10, -1 for all)" },
            },
            required: ["graph_id", "node_id"],
        },
    },
    {
        name: "aggregate_by_name",
        description: "Aggregate all call sites with the same name into a single merged graph. Provide either node_id (uses that node's raw name as the merge key) or pattern (regex — uses the highest-sample match). These are mutually exclusive.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Source graph ID" },
                node_id: { type: "number", description: "Node whose raw name will be used as the merge key (mutually exclusive with pattern)" },
                pattern: { type: "string", description: "Case-insensitive regex — the highest-sample matching node's name is used as the merge key (mutually exclusive with node_id)" },
            },
            required: ["graph_id"],
        },
    },
    {
        name: "icicle_graph",
        description: "Create an inverted (icicle) graph rooted at the given node — leaves become wide root nodes. Provide either root_node_id or pattern (regex — uses the highest-sample match). Returns the ID of the new graph.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Source graph ID" },
                root_node_id: { type: "number", description: "Node ID to use as the root (mutually exclusive with pattern)" },
                pattern: { type: "string", description: "Case-insensitive regex — the highest-sample matching node is used as root (mutually exclusive with root_node_id)" },
            },
            required: ["graph_id"],
        },
    },
    {
        name: "raw_name",
        description: "Return the full raw name of a node. Useful when the display name is truncated and you need the complete identifier.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID from list_graphs" },
                node_id: { type: "number", description: "Node ID" },
            },
            required: ["graph_id", "node_id"],
        },
    },
    {
        name: "hot_path",
        description: "Follow the highest-sample child at each level to find the hot path from a node down to the hottest leaf.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID from list_graphs" },
                node_id: { type: "number", description: "Starting node ID" },
                depth_limit: { type: "number", description: "Maximum depth to follow (default: unlimited)" },
            },
            required: ["graph_id", "node_id"],
        },
    },
    {
        name: "subtree_summary",
        description: "Print a weighted summary of the subtree rooted at a node. Proportionally allocates a line budget to heavier branches, truncating narrow ones. Useful for getting a quick overview of where time is spent under a node.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID from list_graphs" },
                node_id: { type: "number", description: "Root node ID for the summary" },
                max_lines: { type: "number", description: "Maximum number of output lines (default 40)" },
            },
            required: ["graph_id", "node_id"],
        },
    },
    {
        name: "callers",
        description: "Show the caller hierarchy of a function, aggregated across all instances. Like subtree_summary but in reverse — walks up the call stack. Provide either node_id or pattern (regex — uses the highest-sample match). Useful for answering 'who calls X and why?'",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID from list_graphs" },
                node_id: { type: "number", description: "Node whose name to look up callers for (mutually exclusive with pattern)" },
                pattern: { type: "string", description: "Case-insensitive regex — the highest-sample matching node's name is used (mutually exclusive with node_id)" },
                max_lines: { type: "number", description: "Maximum number of output lines (default 40)" },
            },
            required: ["graph_id"],
        },
    },
    {
        name: "delete_graph",
        description: "Delete a derived graph (created by aggregate_by_name or icicle_graph) to free memory. Graphs hold the full node tree in memory — delete them when no longer needed.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID to delete" },
            },
            required: ["graph_id"],
        },
    },
]

function send(msg: object): void {
    process.stdout.write(JSON.stringify(msg) + "\n")
}

function respond(id: string | number, result: unknown): void {
    send({ jsonrpc: "2.0", id, result })
}

function respondError(id: string | number | null, code: number, message: string): void {
    send({ jsonrpc: "2.0", id, error: { code, message } })
}

function handleMessage(line: string): void {
    let msg: any
    try {
        msg = JSON.parse(line)
    } catch {
        respondError(null, -32700, "Parse error")
        return
    }

    const { id, method, params } = msg

    if (method === "initialize") {
        respond(id, {
            protocolVersion: "2024-11-05",
            capabilities: { tools: {} },
            serverInfo: { name: "flamegraph-mcp", version: "1.0.0" },
        })
        return
    }

    if (method === "initialized") {
        return // notification, no response
    }

    if (method === "tools/list") {
        respond(id, { tools: TOOLS })
        return
    }

    if (method === "tools/call") {
        const toolName = params?.name
        const rawArgs = params?.arguments ?? {}
        // Coerce numeric arguments — models sometimes pass them as strings,
        // which turns `id + 1` into string concatenation instead of addition.
        const args: any = Object.fromEntries(
            Object.entries(rawArgs).map(([k, v]) =>
                [k, typeof v === "string" && v !== "" && !isNaN(Number(v)) ? Number(v) : v]
            )
        )
        let text: string
        try {
            if (toolName === "list_graphs") {
                text = toolListGraphs()
            } else if (toolName === "search_by_name") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (!args.pattern) { respondError(id, -32602, "Missing required argument: pattern"); return }
                text = toolSearchByName(args.graph_id, args.pattern, args.limit, args.offset, args.node_id)
            } else if (toolName === "node_details") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolNodeSummary(args.graph_id, args.node_id, args.parent_depth)
            } else if (toolName === "children_of_node") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolChildrenOfNode(args.graph_id, args.node_id, args.limit)
            } else if (toolName === "aggregate_by_name") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null && !args.pattern) { respondError(id, -32602, "Either node_id or pattern must be provided"); return }
                if (args.node_id != null && args.pattern) { respondError(id, -32602, "node_id and pattern are mutually exclusive"); return }
                text = toolAggregateByName(args.graph_id, args.node_id, args.pattern)
            } else if (toolName === "icicle_graph") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.root_node_id == null && !args.pattern) { respondError(id, -32602, "Either root_node_id or pattern must be provided"); return }
                if (args.root_node_id != null && args.pattern) { respondError(id, -32602, "root_node_id and pattern are mutually exclusive"); return }
                text = toolIcicleGraph(args.graph_id, args.root_node_id, args.pattern)
            } else if (toolName === "raw_name") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolRawName(args.graph_id, args.node_id)
            } else if (toolName === "hot_path") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolHotPath(args.graph_id, args.node_id, args.depth_limit ?? null)
            } else if (toolName === "subtree_summary") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolSubtreeSummary(args.graph_id, args.node_id, args.max_lines)
            } else if (toolName === "callers") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null && !args.pattern) { respondError(id, -32602, "Either node_id or pattern must be provided"); return }
                if (args.node_id != null && args.pattern) { respondError(id, -32602, "node_id and pattern are mutually exclusive"); return }
                text = toolCallers(args.graph_id, args.node_id, args.pattern, args.max_lines)
            } else if (toolName === "delete_graph") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                text = toolDeleteGraph(args.graph_id)
            } else {
                respondError(id, -32601, `Unknown tool: ${toolName}`)
                return
            }
        } catch (err: any) {
            respondError(id, -32603, err?.message ?? "Internal error")
            return
        }
        respond(id, { content: [{ type: "text", text }] })
        return
    }

    if (id != null) {
        respondError(id, -32601, `Unknown method: ${method}`)
    }
}

// Top-level await is not supported in CJS; wrap everything async in an IIFE.
;(async () => {

process.stderr.write("Parsing profiles...\n")
for await (const { name, graph } of readEmbeddedStacks()) {
    process.stderr.write(`  Loaded ${name}\n`)
    registerGraph(name, graph)
}
process.stderr.write(`Ready — ${graphMap.size} profile(s) loaded.\n`)

const rl = createInterface({ input: process.stdin, terminal: false })
rl.on("line", (line) => {
    const trimmed = line.trim()
    if (trimmed !== "") handleMessage(trimmed)
})

})() // end async IIFE
