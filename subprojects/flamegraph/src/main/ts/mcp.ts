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

function toolSearchByName(graphId: number, pattern: string, limit = 10, offset = 0): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    let regex: RegExp
    try {
        regex = new RegExp(pattern, "i")
    } catch (e: any) {
        return `Invalid regex: ${e?.message}`
    }

    const total = graph.values[0] ?? 0n
    const matches: Array<{ nodeId: number; value: bigint }> = []
    for (let i = 1; i < graph.nodeCount; i++) {
        if (regex.test(graph.getNodeName(i))) {
            matches.push({ nodeId: i, value: graph.values[i] ?? 0n })
        }
    }

    if (matches.length === 0) {
        return `No nodes matching /${pattern}/i in "${name}".`
    }

    matches.sort((a, b) => Number(b.value - a.value))
    const matchTotal = matches.reduce((sum, { value }) => sum + value, 0n)
    const shown = matches.slice(offset, offset + limit)
    const parentMap = buildParentMap(graph)
    const lines = shown.map(({ nodeId, value }) => {
        const parentId = parentMap[nodeId] ?? -1
        const parentStr = parentId === -1 ? "root" : `[${parentId}] ${graph.getDisplayName(parentId)}`
        return `  [${nodeId}] ${graph.getDisplayName(nodeId)}  ${formatSamples(value, total)}  parent: ${parentStr}`
    })
    const remaining = matches.length - offset - shown.length
    const suffix = remaining > 0 ? `\n  (${remaining} more not shown)` : ""
    return `${matches.length} node(s) matching /${pattern}/i in "${name}" — combined: ${formatSamples(matchTotal, total)} (showing ${shown.length} from offset ${offset}):\n${lines.join("\n")}${suffix}`
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

    const total = graph.values[0] ?? 0n
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
        return `  [${childId}] ${graph.getDisplayName(childId)}  total: ${formatSamples(childValue, total)}  self: ${formatSamples(childSelf, total)}`
    })
    const suffix = limit !== -1 && children.length > limit
        ? `\n  (${children.length - limit} more not shown)`
        : ""
    return `Children of [${nodeId}] "${graph.getDisplayName(nodeId)}" in "${name}" (${shown.length}/${children.length}):\n${lines.join("\n")}${suffix}`
}

function toolAggregateByName(graphId: number, nodeId: number): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    if (nodeId < 0 || nodeId >= graph.nodeCount) {
        return `Node ID ${nodeId} out of range (0–${graph.nodeCount - 1}).`
    }

    const nodeName = graph.getNodeName(nodeId)
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
    const entry = registerGraph(`${name} [merged: ${graph.getDisplayName(nodeId)}]`, newGraph)

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

function toolIcicleGraph(graphId: number, rootNodeId: number): string {
    const resolved = resolveGraph(graphId)
    if (typeof resolved === "string") {
        return resolved
    }
    const { name, graph } = resolved

    if (rootNodeId < 0 || rootNodeId >= graph.nodeCount) {
        return `Node ID ${rootNodeId} out of range (0–${graph.nodeCount - 1}).`
    }

    const raw = graph.toRaw()
    const wg = wasm_icicle_graph(
        raw.childrenOffsets,
        raw.childrenData,
        raw.namesData,
        raw.namesOffsets,
        raw.values,
        rootNodeId,
        nodeCount(raw),
    )
    const newGraph = wasmGraphToGraph(wg)
    const entry = registerGraph(`${name} [icicle: ${graph.getDisplayName(rootNodeId)}]`, newGraph)
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

    const lines = path.map(({ nodeId: nid, value }) => {
        let self = value
        for (const child of graph.getChildren(nid)) {
            self -= graph.values[child] ?? 0n
        }
        return `  [${nid}] ${graph.getDisplayName(nid)}  total: ${formatSamples(value, total)}  self: ${formatSamples(self, total)}`
    })
    return `Hot path from [${nodeId}] in "${name}" (${path.length} nodes shown${depthNote}):\n${lines.join("\n")}`
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

function toolDeleteGraph(graphId: number): string {
    if (!graphMap.has(graphId)) {
        return `Graph ${graphId} not found. Available IDs: ${[...graphMap.keys()].join(", ")}`
    }
    const { name } = graphMap.get(graphId)!
    graphMap.delete(graphId)
    return `Deleted graph ${graphId} "${name}". ${graphMap.size} graph(s) remaining.`
}

const TOOLS = [
    {
        name: "list_graphs",
        description: "List all loaded flamegraph profiles with their IDs, node counts, and total sample counts.",
        inputSchema: { type: "object", properties: {}, required: [] },
    },
    {
        name: "search_by_name",
        description: "Search for nodes whose names match a regex pattern (case-insensitive). Returns matching node IDs sorted by sample count descending.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Graph ID from list_graphs" },
                pattern: { type: "string", description: "Case-insensitive regex pattern to match against node names" },
                limit: { type: "number", description: "Maximum number of results (default 10)" },
                offset: { type: "number", description: "Number of results to skip for pagination (default 0)" },
            },
            required: ["graph_id", "pattern"],
        },
    },
    {
        name: "node_summary",
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
        description: "Aggregate all call sites with the same name as the given node into a single merged graph. The provided node is used only as a name lookup — all matching nodes across the entire graph are merged. Returns the ID of the new graph.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Source graph ID" },
                node_id: { type: "number", description: "Node whose raw name will be used as the merge key" },
            },
            required: ["graph_id", "node_id"],
        },
    },
    {
        name: "icicle_graph",
        description: "Create an inverted (icicle) graph rooted at the given node — leaves become wide root nodes. Returns the ID of the new graph.",
        inputSchema: {
            type: "object",
            properties: {
                graph_id: { type: "number", description: "Source graph ID" },
                root_node_id: { type: "number", description: "Node ID to use as the root of the icicle graph" },
            },
            required: ["graph_id", "root_node_id"],
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
        // Coerce graph_id to number in case the model passes it as a string.
        const args = { ...rawArgs, graph_id: rawArgs.graph_id != null ? Number(rawArgs.graph_id) : rawArgs.graph_id }
        let text: string
        try {
            if (toolName === "list_graphs") {
                text = toolListGraphs()
            } else if (toolName === "search_by_name") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (!args.pattern) { respondError(id, -32602, "Missing required argument: pattern"); return }
                text = toolSearchByName(args.graph_id, args.pattern, args.limit, args.offset)
            } else if (toolName === "node_summary") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolNodeSummary(args.graph_id, args.node_id, args.parent_depth)
            } else if (toolName === "children_of_node") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolChildrenOfNode(args.graph_id, args.node_id, args.limit)
            } else if (toolName === "aggregate_by_name") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolAggregateByName(args.graph_id, args.node_id)
            } else if (toolName === "icicle_graph") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.root_node_id == null) { respondError(id, -32602, "Missing required argument: root_node_id"); return }
                text = toolIcicleGraph(args.graph_id, args.root_node_id)
            } else if (toolName === "raw_name") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolRawName(args.graph_id, args.node_id)
            } else if (toolName === "hot_path") {
                if (args.graph_id == null) { respondError(id, -32602, "Missing required argument: graph_id"); return }
                if (args.node_id == null) { respondError(id, -32602, "Missing required argument: node_id"); return }
                text = toolHotPath(args.graph_id, args.node_id, args.depth_limit ?? null)
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
