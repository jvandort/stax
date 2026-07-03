import {decodeBase64} from "./encoding.ts"
import {
    StacksParser,
    wasm_delete_node,
    wasm_diff_graphs,
    wasm_icicle_graph,
    wasm_merge_children,
    wasm_simplify_graph,
    WasmStackGraph,
} from "@flamegraph-wasm"
import type {DiffGraphData, StackGraphData} from "./stackGraph"
import {nodeCount} from "./stackGraph"

/**
 * Extract typed arrays from a WasmStackGraph and free the Rust-owned memory.
 * Each method call allocates a new JS typed array and copies data out of WASM
 * linear memory into it. These typed arrays can then be zero-copy transferred to
 * the DOM thread.
 */
export const wasmGraphToStackGraph = (wg: WasmStackGraph): StackGraphData => {
    const childrenOffsets = wg.children_offsets()
    const childrenData = wg.children_data()
    const namesData = wg.names_data()
    const namesOffsets = wg.names_offsets()
    const displayNamesData = wg.display_names_data()
    const displayNamesOffsets = wg.display_names_offsets()
    const values = wg.values()
    wg.free()
    return {
        childrenOffsets,
        childrenData,
        namesData,
        namesOffsets,
        displayNamesData,
        displayNamesOffsets,
        values,
    }
}

export const processStream = async (
    stream: ReadableStream<Uint8Array>,
): Promise<StackGraphData> => {
    const parser = new StacksParser("root")
    const reader = stream.getReader()
    while (true) {
        const { done, value } = await reader.read()
        if (value) {
            parser.feed(value)
        }
        if (done) {
            break
        }
    }
    return wasmGraphToStackGraph(parser.finish())
}

export const parseEncodedData = async (
    encodedData: string,
): Promise<StackGraphData> => {
    const CHUNK_SIZE_BYTES = 1024 * 1024
    const encoded = decodeBase64(encodedData)

    let position = 0
    const stream = new ReadableStream<Uint8Array>({
        pull(controller) {
            if (position >= encoded.length) {
                controller.close()
                return
            }
            const end = Math.min(position + CHUNK_SIZE_BYTES, encoded.length)
            controller.enqueue(encoded.subarray(position, end))
            position = end
        },
    }).pipeThrough(
        // DecompressionStream.writable is typed as WritableStream<BufferSource>; cast is safe
        // because we always feed Uint8Array chunks.
        new DecompressionStream("deflate-raw") as unknown as TransformStream<
            Uint8Array,
            Uint8Array
        >,
    )

    return await processStream(stream)
}

export const mergeChildren = (
    graph: StackGraphData,
    nodeName: string,
): StackGraphData => {
    const { childrenOffsets, childrenData, namesData, namesOffsets, values } =
        graph
    return wasmGraphToStackGraph(
        wasm_merge_children(
            childrenOffsets,
            childrenData,
            namesData,
            namesOffsets,
            values,
            nodeName,
        ),
    )
}

export const icicleGraph = (graph: StackGraphData, nodeId: number): StackGraphData => {
    const { childrenOffsets, childrenData, namesData, namesOffsets, values } =
        graph
    return wasmGraphToStackGraph(
        wasm_icicle_graph(
            childrenOffsets,
            childrenData,
            namesData,
            namesOffsets,
            values,
            nodeId,
            nodeCount(graph),
        ),
    )
}

export const deleteNode = (graph: StackGraphData, nodeId: number): StackGraphData => {
    const {
        childrenOffsets,
        childrenData,
        namesData,
        namesOffsets,
        values,
        displayNamesData,
        displayNamesOffsets,
    } = graph
    return wasmGraphToStackGraph(
        wasm_delete_node(
            childrenOffsets,
            childrenData,
            namesData,
            namesOffsets,
            values,
            displayNamesData,
            displayNamesOffsets,
            nodeId,
        ),
    )
}

export const simplifyGraph = (graph: StackGraphData, nodeId: number): StackGraphData => {
    const { childrenOffsets, childrenData, namesData, namesOffsets, values } =
        graph
    const wasmGraph = wasm_simplify_graph(
        childrenOffsets,
        childrenData,
        namesData,
        namesOffsets,
        values,
        nodeId,
    )
    return wasmGraphToStackGraph(wasmGraph)
}


export const diffGraphs = (a: StackGraphData, b: StackGraphData): DiffGraphData => {
    const wasmResult = wasm_diff_graphs(
        a.childrenOffsets,
        a.childrenData,
        a.namesData,
        a.namesOffsets,
        a.values,
        b.childrenOffsets,
        b.childrenData,
        b.namesData,
        b.namesOffsets,
        b.values,
    )
    const diffValues = wasmResult.b_values()
    const graph = wasmGraphToStackGraph(wasmResult.into_graph())
    return { graph, diffValues }
}