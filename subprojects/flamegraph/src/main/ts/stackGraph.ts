const decoder = new TextDecoder()

/**
 * Flat Compressed Sparse Row (CSR) representation of a stack graph, used as
 * the wire format between ust/WASM and JS. This is the shape that worker job
 * types carry.
 *
 * Children are stored in CSR format:
 *   children of node i = childrenData[childrenOffsets[i] .. childrenOffsets[i+1]]
 *
 * Names are interned: each distinct name is stored once in a string table of
 * UTF-8 byte buffers with offset arrays, and each node refers to its name by id:
 *   name of node i = string nameIds[i]
 *   string s = namesData[namesOffsets[s] .. namesOffsets[s+1]]
 * Display names are indexed by the same ids.
 */
export interface StackGraphData {
    childrenOffsets: Int32Array
    childrenData: Int32Array
    nameIds: Int32Array
    namesData: Uint8Array
    namesOffsets: Int32Array
    values: BigInt64Array
    displayNamesData: Uint8Array
    displayNamesOffsets: Int32Array
}

/**
 * Common read-only API shared by StackGraph and DiffGraph. Does not expose
 * values. Use `instanceof` to determine the concrete subtype.
 */
export interface GraphLike {
    nodeCount: number
    getChildren(nodeId: number): Int32Array
    getNodeName(i: number): string
    getDisplayName(i: number): string
    getNodeNameLower(i: number): string
}

/**
 * A stack graph with lazily-decoded string caches, one entry per distinct name.
 */
export class StackGraph implements GraphLike {
    private readonly data: StackGraphData
    private readonly nameCache: (string | undefined)[]
    private readonly displayNameCache: (string | undefined)[]
    private readonly lowerNameCache: (string | undefined)[]

    constructor(data: StackGraphData) {
        this.data = data
        const n = data.namesOffsets.length - 1 // distinct names
        this.nameCache = new Array(n)
        this.displayNameCache = new Array(n)
        this.lowerNameCache = new Array(n)
    }

    get nodeCount(): number {
        return nodeCount(this.data)
    }

    /** Sample counts indexed by node ID. */
    get values(): BigInt64Array {
        return this.data.values
    }

    /** Returns the children of node `nodeId` as a typed array view. */
    getChildren(nodeId: number): Int32Array {
        return this.data.childrenData.subarray(
            this.data.childrenOffsets[nodeId],
            this.data.childrenOffsets[nodeId + 1],
        )
    }

    /** Returns the full name of node `i`. */
    getNodeName(i: number): string {
        const id = this.data.nameIds[i]!
        return (this.nameCache[id] ??= decoder.decode(
            this.data.namesData.subarray(
                this.data.namesOffsets[id],
                this.data.namesOffsets[id + 1],
            ),
        ))
    }

    /** Returns the display name of node `i`. */
    getDisplayName(i: number): string {
        const id = this.data.nameIds[i]!
        return (this.displayNameCache[id] ??= decoder.decode(
            this.data.displayNamesData.subarray(
                this.data.displayNamesOffsets[id],
                this.data.displayNamesOffsets[id + 1],
            ),
        ))
    }

    /** Returns the lowercased name of node `i`. */
    getNodeNameLower(i: number): string {
        return (this.lowerNameCache[this.data.nameIds[i]!] ??= this.getNodeName(i).toLowerCase())
    }

    /**
     * Returns the underlying raw typed-array data, suitable for sending to a
     * worker via postMessage.
     */
    toRaw(): StackGraphData {
        return this.data
    }
}

/** Returns the number of nodes in a raw StackGraphData. Used in the worker where StackGraph is not available. */
export function nodeCount(graph: StackGraphData): number {
    return graph.nameIds.length
}

/**
 * Wire format for a diff of two stack graphs.
 *
 * `graph` is the merged graph, whose `values` hold graph A's per-node
 * inclusive sample counts; `diffValues` holds graph B's. All rendering
 * measures are derived client-side from the two count arrays (see the
 * derived getters on DiffGraph).
 */
export interface DiffGraphData {
    graph: StackGraphData
    diffValues: BigInt64Array
}

interface DiffDerived {
    deltas: BigInt64Array
    selfDeltas: BigInt64Array
}

/**
 * A diff graph with lazily-decoded string caches, backed by a StackGraph.
 */
export class DiffGraph implements GraphLike {
    private readonly inner: StackGraph

    /** Original sample counts from graph A, used for coloring. */
    readonly valuesA: BigInt64Array

    /** Original sample counts from graph B, used for coloring. */
    readonly valuesB: BigInt64Array

    private derived: DiffDerived | null = null

    constructor(data: DiffGraphData) {
        this.inner = new StackGraph(data.graph)
        this.valuesA = data.graph.values
        this.valuesB = data.diffValues
    }

    get nodeCount(): number {
        return this.inner.nodeCount
    }

    /** Signed inclusive delta per node: `b - a`. */
    get deltas(): BigInt64Array {
        return this.getDerived().deltas
    }

    /** Signed self-time delta per node: `delta - Σ delta(children)`. */
    get selfDeltas(): BigInt64Array {
        return this.getDerived().selfDeltas
    }

    private getDerived(): DiffDerived {
        if (this.derived) return this.derived

        const n = this.nodeCount
        const deltas = new BigInt64Array(n)
        for (let i = 0; i < n; i++) {
            deltas[i] = this.valuesB[i]! - this.valuesA[i]!
        }

        const selfDeltas = new BigInt64Array(n)
        for (let i = 0; i < n; i++) {
            let self = deltas[i]!
            for (const c of this.getChildren(i)) {
                self -= deltas[c]!
            }
            selfDeltas[i] = self
        }

        this.derived = { deltas, selfDeltas }
        return this.derived
    }

    getChildren(nodeId: number): Int32Array {
        return this.inner.getChildren(nodeId)
    }

    getNodeName(i: number): string {
        return this.inner.getNodeName(i)
    }

    getDisplayName(i: number): string {
        return this.inner.getDisplayName(i)
    }

    getNodeNameLower(i: number): string {
        return this.inner.getNodeNameLower(i)
    }
}
