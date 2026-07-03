import init from "@flamegraph-wasm"
import {
    parseEncodedData,
    processStream,
    mergeChildren,
    icicleGraph,
    deleteNode,
    simplifyGraph,
    diffGraphs
} from "./wasmBridge.ts"
import type { DiffGraphData, StackGraphData } from "./stackGraph"

let resolveWasmReady: () => void
const wasmReady = new Promise<void>((resolve) => {
    resolveWasmReady = resolve
})

export interface ParseEncodedDataJob {
    encodedData: string
    type: "parseEncodedData"
}

export interface InitWorkerJob {
    module: WebAssembly.Module
    type: "initWasm"
}

export interface ParseStreamJob {
    stream: ReadableStream<Uint8Array>
    type: "parseStream"
}

export interface MergeChildrenJob {
    nodeName: string
    graph: StackGraphData
    type: "mergeChildren"
}

export interface IcicleGraphJob {
    nodeId: number
    graph: StackGraphData
    type: "icicleGraph"
}

export interface DeleteNodeJob {
    nodeId: number
    graph: StackGraphData
    type: "deleteNode"
}

export interface SimplifyGraphJob {
    nodeId: number
    graph: StackGraphData
    type: "simplifyGraph"
}

export interface DiffGraphsJob {
    graphA: StackGraphData
    graphB: StackGraphData
    type: "diffGraphs"
}

export type Job =
    | InitWorkerJob
    | ParseStreamJob
    | ParseEncodedDataJob
    | MergeChildrenJob
    | IcicleGraphJob
    | DeleteNodeJob
    | SimplifyGraphJob
    | DiffGraphsJob

export interface WorkerParams {
    job: Job
}

export type WorkerResult =
    | { graph: StackGraphData }
    | { diffGraph: DiffGraphData }

export interface WorkerSuccess {
    result: WorkerResult
}

export interface WorkerFailure {
    error: {
        message: string
        stack?: string
    }
}

export type WorkerResponse = WorkerSuccess | WorkerFailure

const stackGraphTransferables = (g: StackGraphData): Transferable[] => [
    g.childrenOffsets.buffer,
    g.childrenData.buffer,
    g.namesData.buffer,
    g.namesOffsets.buffer,
    g.displayNamesData.buffer,
    g.displayNamesOffsets.buffer,
    g.values.buffer,
]

const process = async (job: Job): Promise<WorkerResult> => {
    if (job.type == "parseStream") {
        return { graph: await processStream(job.stream) }
    } else if (job.type == "mergeChildren") {
        return { graph: mergeChildren(job.graph, job.nodeName) }
    } else if (job.type == "icicleGraph") {
        return { graph: icicleGraph(job.graph, job.nodeId) }
    } else if (job.type == "parseEncodedData") {
        return { graph: await parseEncodedData(job.encodedData) }
    } else if (job.type == "deleteNode") {
        return { graph: deleteNode(job.graph, job.nodeId) }
    } else if (job.type == "simplifyGraph") {
        return { graph: simplifyGraph(job.graph, job.nodeId) }
    } else if (job.type == "diffGraphs") {
        return { diffGraph: diffGraphs(job.graphA, job.graphB) }
    }

    throw new Error("Unknown job type")
}

const tryProcess = async (job: Job): Promise<WorkerResponse> => {
    try {
        const result = await process(job)
        return { result }
    } catch (error) {
        const message =
            error instanceof Error
                ? error.message
                : typeof error === "string"
                  ? error
                  : "Unknown error"
        const stack = error instanceof Error ? error.stack : undefined
        return {
            error: { message, stack },
        }
    }
}

self.onmessage = async (event: MessageEvent<WorkerParams>) => {
    if (event.data.job.type == "initWasm") {
        try {
            await init({ module_or_path: event.data.job.module })
            resolveWasmReady()
        } catch (error) {
            console.error("Failed to initialize worker: " + error)
        }
        return
    }

    await wasmReady

    const response = await tryProcess(event.data.job)
    if ("result" in response) {
        const result = response.result
        if ("graph" in result) {
            self.postMessage(response, stackGraphTransferables(result.graph))
        } else {
            self.postMessage(response, [
                ...stackGraphTransferables(result.diffGraph.graph),
                result.diffGraph.diffValues.buffer,
            ])
        }
    } else {
        self.postMessage(response, [])
    }
}
