const decoder = new TextDecoder()

/**
 * Flat Compressed Sparse Row (CSR) representation of a stack graph, used as
 * the wire format between ust/WASM and JS. This is the shape that worker job
 * types carry.
 *
 * Children are stored in CSR format:
 *   children of node i = childrenData[childrenOffsets[i] .. childrenOffsets[i+1]]
 *
 * Names and display names are UTF-8 byte buffers with parallel offset arrays:
 *   name of node i = namesData[namesOffsets[i] .. namesOffsets[i+1]]
 */
export interface StackGraphData {
    childrenOffsets: Int32Array
    childrenData: Int32Array
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
 * A stack graph with lazily-decoded string caches.
 */
export class StackGraph implements GraphLike {
    private readonly data: StackGraphData
    private readonly nameCache: (string | undefined)[]
    private readonly displayNameCache: (string | undefined)[]
    private readonly lowerNameCache: (string | undefined)[]

    constructor(data: StackGraphData) {
        this.data = data
        const n = data.namesOffsets.length - 1
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
        return (this.nameCache[i] ??= decoder.decode(
            this.data.namesData.subarray(
                this.data.namesOffsets[i],
                this.data.namesOffsets[i + 1],
            ),
        ))
    }

    /** Returns the display name of node `i`. */
    getDisplayName(i: number): string {
        return (this.displayNameCache[i] ??= decoder.decode(
            this.data.displayNamesData.subarray(
                this.data.displayNamesOffsets[i],
                this.data.displayNamesOffsets[i + 1],
            ),
        ))
    }

    /** Returns the lowercased name of node `i`. */
    getNodeNameLower(i: number): string {
        return (this.lowerNameCache[i] ??= this.getNodeName(i).toLowerCase())
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
    return graph.namesOffsets.length - 1
}

/**
 * Wire format for a diff of two stack graphs.
 *
 * `graph.values` holds bottom-up `|b - a|` values computed by the WASM diff.
 * The renderer no longer uses them; all rendering measures are derived
 * client-side from `aValues`/`bValues` (see the derived getters on DiffGraph).
 *
 * `aValues` and `bValues` hold the original per-node inclusive sample counts
 * from each input graph.
 */
export interface DiffGraphData {
    graph: StackGraphData
    aValues: BigInt64Array
    bValues: BigInt64Array
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
        this.valuesA = data.aValues
        this.valuesB = data.bValues
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
