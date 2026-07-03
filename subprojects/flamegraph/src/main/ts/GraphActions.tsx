import React from "react"
import type { GraphState } from "./useGraphTabs"
import { Row } from "./containers.tsx"
import { getGraph } from "./graphStore"
import { DiffGraph, StackGraph } from "./stackGraph"
import type { DiffRenderMode, DiffRenderOptions } from "./FlamegraphNode"

const MODE_BUTTONS: { mode: DiffRenderMode; label: string; title: string }[] = [
    {
        mode: "delta",
        label: "Delta",
        title: "Width = |net Δ|. Subtrees whose changes cancel out disappear.",
    },
    {
        mode: "regressions",
        label: "Slower",
        title: "Only subtrees that got net slower. Drill into a subtree to reveal slowdowns hidden inside net-faster areas.",
    },
    {
        mode: "improvements",
        label: "Faster",
        title: "Only subtrees that got net faster. Drill into a subtree to reveal speedups hidden inside net-slower areas.",
    },
]

export const GraphActions: React.FC<{
    tabId: string | null
    graphState: GraphState | null
    goBack: (tabId: string) => void
    goForward: (tabId: string) => void
    setRootNode: (tabId: string, nodeId: number) => void
    showMergedSubgraph: (tabId: string, nodeId: number) => void
    showIcicleGraph: (tabId: string, nodeId: number) => void
    showSimplifiedGraph: (tabId: string, nodeId: number) => void
    setMutable: (tabId: string, mutable: boolean) => void
    diffRenderOptions: DiffRenderOptions
    onChangeDiffRenderOptions: (options: DiffRenderOptions) => void
}> = ({
    tabId,
    graphState,
    goBack,
    goForward,
    setRootNode,
    showMergedSubgraph,
    showIcicleGraph,
    showSimplifiedGraph,
    setMutable,
    diffRenderOptions,
    onChangeDiffRenderOptions,
}) => {
    const historyEntry = graphState?.history[graphState.historyIndex]
    const rootNode = historyEntry?.rootNode ?? 0
    const canGoBack = graphState ? graphState.historyIndex > 0 : false
    const canGoForward = graphState
        ? graphState.historyIndex < graphState.history.length - 1
        : false
    const isMutable = graphState?.mutable ?? false
    const graph = tabId != null ? getGraph(tabId) : undefined
    const isStackGraph = graph instanceof StackGraph
    const isDiffGraph = graph instanceof DiffGraph

    const setOption = <K extends keyof DiffRenderOptions>(
        key: K,
        value: DiffRenderOptions[K],
    ) => onChangeDiffRenderOptions({ ...diffRenderOptions, [key]: value })

    return (
        <Row style={{ gap: 5, justifyContent: "flex-end" }}>
            {isDiffGraph && (
                <button
                    aria-pressed={diffRenderOptions.swapped}
                    onClick={() =>
                        // The frames on screen keep their exact widths and
                        // positions; only the interpretation flips. In a
                        // signed view that means the selected view follows
                        // the content: what was "Slower" is, under the
                        // swapped reading, the same frames now "Faster".
                        onChangeDiffRenderOptions({
                            swapped: !diffRenderOptions.swapped,
                            mode:
                                diffRenderOptions.mode === "regressions"
                                    ? "improvements"
                                    : diffRenderOptions.mode === "improvements"
                                      ? "regressions"
                                      : "delta",
                        })
                    }
                    title="Reinterpret A as B and B as A. The view keeps showing the same frames; red/green flip."
                >
                    Swap
                </button>
            )}
            {isDiffGraph &&
                MODE_BUTTONS.map(({ mode, label, title }) => (
                    <button
                        key={mode}
                        aria-pressed={diffRenderOptions.mode === mode}
                        onClick={() => setOption("mode", mode)}
                        title={title}
                    >
                        {label}
                    </button>
                ))}
            {isStackGraph && (
                <button
                    onClick={() =>
                        tabId &&
                        graphState &&
                        showMergedSubgraph(tabId, rootNode)
                    }
                    disabled={!graphState || rootNode === 0}
                >
                    Merge
                </button>
            )}
            {isStackGraph && (
                <button
                    onClick={() =>
                        tabId && graphState && showIcicleGraph(tabId, rootNode)
                    }
                    disabled={!graphState}
                >
                    Icicle
                </button>
            )}
            {isStackGraph && (
                <button
                    onClick={() =>
                        tabId &&
                        graphState &&
                        showSimplifiedGraph(tabId, rootNode)
                    }
                    disabled={!graphState}
                >
                    Simplify
                </button>
            )}
            {isStackGraph && (
                <button onClick={() => tabId && setMutable(tabId, !isMutable)}>
                    {isMutable ? "Freeze" : "Mutate"}
                </button>
            )}
            <button
                onClick={() => tabId && goBack(tabId)}
                disabled={!tabId || !canGoBack}
            >
                &larr; Back
            </button>
            <button
                onClick={() => tabId && goForward(tabId)}
                disabled={!tabId || !canGoForward}
            >
                Forward &rarr;
            </button>
            <button
                onClick={() => tabId && setRootNode(tabId, 0)}
                disabled={!tabId || rootNode === 0}
            >
                Reset
            </button>
        </Row>
    )
}
