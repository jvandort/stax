import init from "@flamegraph-wasm"
import {
    parseEncodedData,
    processStream,
    mergeChildren,
    icicleGraph,
    deleteNode,
} from "./wasmBridge.ts"
import type { StackGraph } from "./stackGraph"

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
    graph: StackGraph
    type: "mergeChildren"
}

export interface IcicleGraphJob {
    nodeId: number
    graph: StackGraph
    type: "icicleGraph"
}

export interface DeleteNodeJob {
    nodeId: number
    graph: StackGraph
    type: "deleteNode"
}

export type Job =
    | InitWorkerJob
    | ParseStreamJob
    | ParseEncodedDataJob
    | MergeChildrenJob
    | IcicleGraphJob
    | DeleteNodeJob

export interface WorkerParams {
    job: Job
}

export interface WorkerResult {
    graph: StackGraph
}

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
        const g = response.result.graph
        self.postMessage(response, [
            g.childrenOffsets.buffer,
            g.childrenData.buffer,
            g.namesData.buffer,
            g.namesOffsets.buffer,
            g.displayNamesData.buffer,
            g.displayNamesOffsets.buffer,
            g.values.buffer,
        ])
    } else {
        self.postMessage(response, [])
    }
}
