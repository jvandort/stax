import { useCallback } from "react"
import type { GraphState, RunJob } from "./useGraphTabs"
import { getGraph } from "./graphStore"
import { type GraphLike, StackGraph } from "./stackGraph"

export function useGraphMutation(
    updateGraphState: (
        id: string,
        updater: (prev: GraphState) => GraphState,
    ) => void,
    replaceTabGraph: (oldId: string, newGraph: GraphLike) => void,
    runJob: RunJob,
) {
    const setMutable = useCallback(
        (tabId: string, mutable: boolean) => {
            updateGraphState(tabId, (gs) => ({ ...gs, mutable }))
        },
        [updateGraphState],
    )

    const deleteNode = useCallback(
        async (tabId: string, nodeId: number) => {
            const graph = getGraph(tabId)
            if (!graph) return
            if (!(graph instanceof StackGraph)) throw new Error("Not supported for diff graphs")
            const result = await runJob(
                "deleteNode",
                {
                    job: {
                        type: "deleteNode",
                        nodeId,
                        graph: graph.toRaw(),
                    },
                },
                [],
            )
            if ("result" in result && "graph" in result.result) {
                replaceTabGraph(tabId, new StackGraph(result.result.graph))
            }
        },
        [replaceTabGraph, runJob],
    )

    return { setMutable, deleteNode }
}
