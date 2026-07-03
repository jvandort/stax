import { useCallback, useState } from "react"
import type { Job, WorkerParams, WorkerResponse } from "./worker"
import DataWorker from "./worker?worker&inline"
import useWorkerPool from "./useWorkerPool.ts"
import { COORDINATE_WIDTH } from "./FlamegraphNode"
import {
    generateGraphId,
    getGraph,
    removeGraph,
    storeGraph,
} from "./graphStore"
import { DiffGraph, type GraphLike, StackGraph } from "./stackGraph"

export interface HistoryEntry {
    rootNode: number
    viewLeft: number
    viewRight: number
    scrollTop?: number
}

export interface GraphState {
    history: HistoryEntry[]
    historyIndex: number
    mutable?: boolean
}

export interface WorkerState {
    name: string
    error?: {
        message: string
        stack?: string
    }
    progress?: string
    graph?: GraphState
}

export type RunJob = (
    id: string,
    params: WorkerParams,
    transfer?: Transferable[],
) => Promise<WorkerResponse>

export function useGraphTabs() {
    const runJob: RunJob = useWorkerPool<string, WorkerParams, WorkerResponse>(
        DataWorker,
    )

    const [selectedTab, setSelectedTab] = useState<string | null>(null)
    const [allTabData, setAllTabData] = useState<Map<string, WorkerState>>(
        new Map(),
    )

    /** Sets tab state, preserving the existing `name` (or defaulting to `id`). */
    const setTabData = useCallback(
        (id: string, newData: Omit<WorkerState, "name">) => {
            setAllTabData((oldData) => {
                const updated = new Map(oldData)
                const name = oldData.get(id)?.name ?? id
                updated.set(id, { ...newData, name })
                return updated
            })
            setSelectedTab((current) => current ?? id)
        },
        [],
    )

    const updateGraphState = useCallback(
        (id: string, updater: (prev: GraphState) => GraphState) => {
            setAllTabData((oldData) => {
                const tabState = oldData.get(id)
                if (!tabState?.graph) {
                    return oldData
                }
                const updated = new Map(oldData)
                updated.set(id, { ...tabState, graph: updater(tabState.graph) })
                return updated
            })
        },
        [],
    )

    const deleteTab = useCallback((id: string) => {
        setAllTabData((oldData) => {
            if (oldData.get(id)?.graph) {
                removeGraph(id)
            }
            const updated = new Map(oldData)
            updated.delete(id)
            return updated
        })
        setSelectedTab((current) => (current === id ? null : current))
    }, [])

    /**
     * Replaces a mutated graph: stores the new graph under a fresh ID, removes
     * the old one, and renames the tab entry in-place (preserving display name
     * and history position). Used by graph mutation operations (e.g. deleteNode).
     */
    const replaceTabGraph = useCallback(
        (oldId: string, newGraph: GraphLike) => {
            const newId = generateGraphId()
            storeGraph(newId, newGraph)
            removeGraph(oldId)
            setAllTabData((oldData) => {
                const tabState = oldData.get(oldId)
                if (!tabState) return oldData
                const updated = new Map<string, WorkerState>()
                for (const [k, v] of oldData) {
                    updated.set(k === oldId ? newId : k, v)
                }
                return updated
            })
            setSelectedTab((current) => (current === oldId ? newId : current))
        },
        [],
    )

    const submitJob = useCallback(
        async (id: string, job: Job, transfer: Transferable[]) => {
            setTabData(id, { progress: "Crunching the numbers..." })
            const params: WorkerParams = { job }
            try {
                const result = await runJob(id, params, transfer)
                if ("error" in result) {
                    setTabData(id, { error: result.error })
                } else if ("result" in result) {
                    const workerResult = result.result
                    if ("graph" in workerResult) {
                        storeGraph(id, new StackGraph(workerResult.graph))
                    } else {
                        storeGraph(id, new DiffGraph(workerResult.diffGraph))
                    }
                    setTabData(id, {
                        graph: {
                            history: [
                                {
                                    rootNode: 0,
                                    viewLeft: 0,
                                    viewRight: COORDINATE_WIDTH,
                                },
                            ],
                            historyIndex: 0,
                        },
                    })
                }
            } catch (err) {
                setTabData(id, {
                    error: {
                        message:
                            err instanceof Error ? err.message : String(err),
                    },
                })
            }
        },
        [runJob, setTabData],
    )

    const showMergedSubgraph = useCallback(
        (tabId: string, nodeId: number) => {
            const graph = getGraph(tabId)
            if (!graph) return
            if (!(graph instanceof StackGraph)) throw new Error("Not supported for diff graphs")
            const nodeName = graph.getNodeName(nodeId)
            const newTabId = `${tabId}:merge:${nodeName}`
            submitJob(
                newTabId,
                { type: "mergeChildren", nodeName, graph: graph.toRaw() },
                [],
            )
            setSelectedTab(newTabId)
        },
        [submitJob],
    )

    const showIcicleGraph = useCallback(
        (tabId: string, nodeId: number) => {
            const graph = getGraph(tabId)
            if (!graph) return
            if (!(graph instanceof StackGraph)) throw new Error("Not supported for diff graphs")
            const nodeName = graph.getNodeName(nodeId)
            const newTabId = `${tabId}:icicle:${nodeName}`
            submitJob(
                newTabId,
                { type: "icicleGraph", nodeId, graph: graph.toRaw() },
                [],
            )
            setSelectedTab(newTabId)
        },
        [submitJob],
    )

    const showSimplifiedGraph = useCallback(
        (tabId: string, nodeId: number) => {
            const graph = getGraph(tabId)
            if (!graph) return
            if (!(graph instanceof StackGraph)) throw new Error("Not supported for diff graphs")
            const nodeName = graph.getNodeName(nodeId)
            const newTabId = `${tabId}:simplified:${nodeName}`
            submitJob(
                newTabId,
                { type: "simplifyGraph", nodeId, graph: graph.toRaw() },
                [],
            )
            setSelectedTab(newTabId)
        },
        [submitJob],
    )

    const diffGraphs = useCallback(
        (aTabId: string, bTabId: string) => {
            const aGraph = getGraph(aTabId)
            const bGraph = getGraph(bTabId)
            if (!aGraph || !bGraph) return
            if (!(aGraph instanceof StackGraph) || !(bGraph instanceof StackGraph)) throw new Error("Not supported for diff graphs")
            const newTabId = `${aTabId} vs ${bTabId}`
            submitJob(
                newTabId,
                {
                    type: "diffGraphs",
                    graphA: aGraph.toRaw(),
                    graphB: bGraph.toRaw(),
                },
                [],
            )
            setSelectedTab(newTabId)
        },
        [submitJob],
    )

    return {
        runJob,
        allTabData,
        selectedTab,
        setSelectedTab,
        updateGraphState,
        replaceTabGraph,
        deleteTab,
        submitJob,
        showMergedSubgraph,
        showIcicleGraph,
        showSimplifiedGraph,
        diffGraphs,
    }
}
