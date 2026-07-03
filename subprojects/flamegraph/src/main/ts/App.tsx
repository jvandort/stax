import React, { useCallback, useEffect, useRef, useState } from "react"
import { Flamegraph } from "./Flamegraph"
import {
    COORDINATE_WIDTH,
    DEFAULT_DIFF_RENDER_OPTIONS,
    type DiffRenderOptions,
} from "./FlamegraphNode"
import { RangeSlider } from "./Sliders"
import { Row, Stack } from "./containers.tsx"
import type { ColorSettings } from "./color"
import { useGraphTabs } from "./useGraphTabs"
import { useGraphHistory } from "./useGraphHistory"
import { useGraphMutation } from "./useGraphMutation"
import { GraphPicker } from "./GraphPicker"
import { ColorPicker } from "./ColorPicker"
import { GraphActions } from "./GraphActions"
import { SearchPanel } from "./SearchPanel"
import { ResizablePanel } from "./ResizablePanel"
import { decodeBase64 } from "./encoding.ts"
import { McpInstructionsOverlay } from "./McpInstructionsOverlay"
import MCP_TEMPLATE from "virtual:mcp-template"

type OpenPanel = "graphs" | "colors" | null

const PANEL_STYLE = {
    background: "rgba(0, 0, 0, 0.6)",
    padding: 10,
    paddingTop: 0,
    gap: 15,
    pointerEvents: "auto" as const,
}

interface EmbeddedStack {
    name: string
    encodedData: string
}

const App = (): React.JSX.Element => {
    const {
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
    } = useGraphTabs()

    const [embeddedStacks, setEmbeddedStacks] = useState<EmbeddedStack[]>([])
    const [isMcpDownloading, setIsMcpDownloading] = useState(false)
    const [mcpFilename, setMcpFilename] = useState<string | null>(null)
    const [showMcpPopup, setShowMcpPopup] = useState(false)

    // Ref so the parsed stacks survive React strict mode's effect double-invocation.
    // Strict mode intentionally kills and recreates the worker pool between the two
    // runs, so the first run's in-flight parseEncodedData job is lost. On the second
    // run the DOM elements are already removed, but the ref still holds the data so
    // we can re-submit to the healthy new pool.
    const embeddedStacksRef = useRef<EmbeddedStack[]>([])

    useEffect(() => {
        if (embeddedStacksRef.current.length > 0) {
            // Second invocation (strict mode): re-submit to the new worker pool.
            embeddedStacksRef.current.forEach(({ name, encodedData }) => {
                submitJob(name, { type: "parseEncodedData", encodedData }, [])
            })
            return
        }

        const namesEl = document.getElementById("embedded-stacks-names")
        if (!namesEl) return

        const stackNames = namesEl.innerHTML.trim().split(",").map(atob)
        const stacks = stackNames.map((name, i) => {
            const template = document.getElementById(
                `embedded-stacks-${i}`,
            ) as HTMLTemplateElement
            if (!template) {
                throw new Error(
                    `Missing embedded stack template element: embedded-stacks-${i}`,
                )
            }
            const encodedData = template.content.textContent
            if (encodedData === null) {
                throw new Error(
                    `Embedded stack template embedded-stacks-${i} has no text content`,
                )
            }
            template.remove()
            return { name, encodedData }
        })
        stacks.forEach(({ name, encodedData }) => {
            submitJob(name, { type: "parseEncodedData", encodedData }, [])
        })
        embeddedStacksRef.current = stacks
        setEmbeddedStacks(stacks)
        namesEl.remove()
    }, [submitJob])

    const handleMcpDownload = useCallback(async () => {
        if (embeddedStacks.length === 0) {
            return
        }

        if (mcpFilename !== null) {
            setShowMcpPopup(true)
            return
        }

        setIsMcpDownloading(true)
        const templateBytes = decodeBase64(MCP_TEMPLATE)
        const ds = new DecompressionStream("gzip")
        const writer = ds.writable.getWriter()
        // Both code paths in decodeBase64 allocate a plain ArrayBuffer (never
        // SharedArrayBuffer), so the cast is safe even though the return type
        // is Uint8Array<ArrayBufferLike>.
        void writer.write(templateBytes as unknown as Uint8Array<ArrayBuffer>)
        void writer.close()

        const reader = ds.readable.getReader()
        const chunks: Uint8Array[] = []
        while (true) {
            const { done, value } = await reader.read()
            if (done) {
                break
            }
            if (value) {
                chunks.push(value)
            }
        }

        let totalLength = 0
        for (const chunk of chunks) {
            totalLength += chunk.length
        }
        const merged = new Uint8Array(totalLength)
        let offset = 0
        for (const chunk of chunks) {
            merged.set(chunk, offset)
            offset += chunk.length
        }

        const templateJs = new TextDecoder("utf-8").decode(merged)
        const wasmBase64 = await window.WASM_BASE64

        const blobParts: BlobPart[] = [
            "#!/usr/bin/env node\n",
            `const __WASM_BASE64__="`,
            wasmBase64,
            `";\n`,
            templateJs,
            "\n// EMBEDDED_DATA_START\n",
        ]
        for (const { name, encodedData } of embeddedStacks) {
            blobParts.push(
                `// STACKS_NAME ${name}\n`,
                "// STACKS_DATA ",
                encodedData,
                "\n",
            )
        }

        const filename = `flamegraph-${crypto.randomUUID()}.js`
        const blob = new Blob(blobParts, { type: "application/javascript" })
        const url = URL.createObjectURL(blob)
        const a = document.createElement("a")
        a.href = url
        a.download = filename
        a.click()
        URL.revokeObjectURL(url)
        setMcpFilename(filename)
        setShowMcpPopup(true)
        setIsMcpDownloading(false)
    }, [embeddedStacks, mcpFilename])

    const selectedTabData = selectedTab ? allTabData.get(selectedTab) : null
    const graphState = selectedTabData?.graph ?? null

    const { rootNode, viewLeft, viewRight } = graphState
        ? graphState.history[graphState.historyIndex]!
        : { rootNode: 0, viewLeft: 0, viewRight: COORDINATE_WIDTH }

    const { setRootNode, updateZoom, goBack, goForward } =
        useGraphHistory(updateGraphState)

    const { setMutable, deleteNode } = useGraphMutation(
        updateGraphState,
        replaceTabGraph,
        runJob,
    )

    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (!e.metaKey || !selectedTab) return
            if (e.key === "[") {
                e.preventDefault()
                goBack(selectedTab)
            } else if (e.key === "]") {
                e.preventDefault()
                goForward(selectedTab)
            }
        }
        window.addEventListener("keydown", handleKeyDown)
        return () => window.removeEventListener("keydown", handleKeyDown)
    }, [goBack, goForward, selectedTab])

    const [colorSettings, setColorSettings] = useState<ColorSettings>({
        center: 98,
        width: 100,
        amount: 1.67,
        distribution: 1199,
    })

    const [openPanel, setOpenPanel] = useState<OpenPanel>("graphs")
    const [diffRenderOptions, setDiffRenderOptions] =
        useState<DiffRenderOptions>(DEFAULT_DIFF_RENDER_OPTIONS)
    const togglePanel = (panel: Exclude<OpenPanel, null>) =>
        setOpenPanel((current) => (current === panel ? null : panel))

    const [searchOpen, setSearchOpen] = useState(false)
    const [searchQuery, setSearchQuery] = useState("")

    const handleScrollChange = useCallback(
        (scrollTop: number) => {
            if (selectedTab) {
                updateGraphState(selectedTab, (prev) => {
                    const newHistory = [...prev.history]
                    newHistory[prev.historyIndex] = {
                        ...newHistory[prev.historyIndex]!,
                        scrollTop,
                    }
                    return { ...prev, history: newHistory }
                })
            }
        },
        [selectedTab, updateGraphState],
    )

    return (
        <Flamegraph
            graphId={selectedTab}
            rootNode={rootNode}
            setRootNode={(nodeId) =>
                selectedTab && setRootNode(selectedTab, nodeId)
            }
            viewLeft={viewLeft}
            viewRight={viewRight}
            onUpdateZoom={(left, right) =>
                selectedTab && updateZoom(selectedTab, left, right)
            }
            isMutable={graphState?.mutable ?? false}
            onDeleteNode={(nodeId) =>
                selectedTab && deleteNode(selectedTab, nodeId)
            }
            colorSettings={colorSettings}
            initialScrollTop={
                graphState?.history[graphState.historyIndex]?.scrollTop
            }
            onScrollChange={handleScrollChange}
            searchQuery={searchQuery || undefined}
            diffRenderOptions={diffRenderOptions}
        >
            {/* Wrapper provides the positioning context for the center overlay,
                spanning the full flamegraph area including the range slider. */}
            <Stack wide style={{ position: "relative", flex: 1, minHeight: 0 }}>
                {/* Center: empty state / progress / errors — absolutely covers the
                    entire flamegraph overlay. Rendered first so panels in DOM
                    order paint on top without needing explicit z-index. */}
                <Stack
                    style={{
                        position: "absolute",
                        inset: 0,
                        justifyContent: "center",
                        alignItems: "center",
                        pointerEvents: "none",
                    }}
                >
                    {!selectedTab && (
                        <div style={{ opacity: 0.5 }}>
                            Load a -stacks.txt file to begin
                        </div>
                    )}
                    {selectedTabData?.progress && (
                        <div>{selectedTabData.progress}</div>
                    )}
                    {selectedTabData?.error && (
                        <>
                            <div>Error: {selectedTabData.error.message}</div>
                            <div>
                                {selectedTabData.error.stack
                                    ?.split("\n")
                                    .map((line, index) => (
                                        <div key={index}>{line}</div>
                                    ))}
                            </div>
                        </>
                    )}
                </Stack>

                <Row
                    style={{
                        background: "rgba(0, 0, 0, 0.6)",
                        padding: "10px",
                        height: "40px",
                        pointerEvents: "auto",
                        alignItems: "center",
                        gap: "8px",
                    }}
                >
                    <button
                        onClick={() => {
                            void handleMcpDownload()
                        }}
                        disabled={
                            embeddedStacks.length === 0 || isMcpDownloading
                        }
                        title="Download MCP server for this flamegraph"
                    >
                        MCP
                    </button>
                    <RangeSlider
                        min={0}
                        max={COORDINATE_WIDTH}
                        valueLeft={viewLeft}
                        valueRight={viewRight}
                        onChange={(left, right) =>
                            selectedTab && updateZoom(selectedTab, left, right)
                        }
                        disabled={!graphState}
                    />
                </Row>

                <Row
                    wide
                    style={{ flex: 1, minHeight: 0, alignItems: "stretch" }}
                >
                    {/* Left panel: graphs / colors. */}
                    <ResizablePanel
                        edge="right"
                        open={openPanel !== null}
                        initialWidth={450}
                        minWidth={200}
                        style={{ alignSelf: "stretch", maxWidth: "50%" }}
                        panelStyle={PANEL_STYLE}
                    >
                        <Row style={{ gap: 5 }}>
                            <button
                                aria-pressed={openPanel === "graphs"}
                                onClick={() => togglePanel("graphs")}
                            >
                                Graphs
                            </button>
                            <button
                                aria-pressed={openPanel === "colors"}
                                onClick={() => togglePanel("colors")}
                            >
                                Colors
                            </button>
                        </Row>
                        {openPanel !== null && (
                            <Stack
                                style={{
                                    flex: 1,
                                    minHeight: 0,
                                    overflowY: "auto",
                                }}
                            >
                                {openPanel === "graphs" && (
                                    <GraphPicker
                                        tabs={[...allTabData.entries()].map(
                                            ([id, state]) => ({
                                                id,
                                                name: state.name,
                                            }),
                                        )}
                                        selectedTab={selectedTab}
                                        onSelectTab={setSelectedTab}
                                        onDeleteTab={deleteTab}
                                        onFileSelected={(file) => {
                                            const stream = file.stream()
                                            submitJob(
                                                file.name,
                                                { type: "parseStream", stream },
                                                [stream],
                                            )
                                            setSelectedTab(file.name)
                                        }}
                                        onDiff={diffGraphs}
                                    />
                                )}
                                {openPanel === "colors" && (
                                    <ColorPicker
                                        colorSettings={colorSettings}
                                        onColorChange={setColorSettings}
                                    />
                                )}
                            </Stack>
                        )}
                    </ResizablePanel>

                    {/* Right side: GraphActions above the search panel. The column
                        stretches to full row height when search is open so the
                        ResizablePanel can fill down to NodeDetails. */}
                    <Stack
                        style={{
                            position: "relative",
                            alignItems: "flex-end",
                            alignSelf: searchOpen ? "stretch" : "flex-start",
                            marginLeft: "auto",
                            maxWidth: "50%",
                        }}
                    >
                        {/* GraphActions — own natural width, not resizable */}
                        <Stack style={{ ...PANEL_STYLE, flexShrink: 0 }}>
                            <GraphActions
                                tabId={selectedTab}
                                graphState={graphState}
                                goBack={goBack}
                                goForward={goForward}
                                setRootNode={setRootNode}
                                showMergedSubgraph={showMergedSubgraph}
                                showIcicleGraph={showIcicleGraph}
                                showSimplifiedGraph={showSimplifiedGraph}
                                setMutable={setMutable}
                                diffRenderOptions={diffRenderOptions}
                                onChangeDiffRenderOptions={setDiffRenderOptions}
                            />
                        </Stack>

                        {/* Search panel — resizable width, content-driven height
                            capped at the remaining column height when open.
                            maxWidth: 100% prevents the stored width from visually
                            overflowing the column when CSS clamps the column. */}
                        <ResizablePanel
                            edge="left"
                            open={searchOpen}
                            initialWidth={350}
                            minWidth={200}
                            style={{
                                flex: searchOpen ? 1 : undefined,
                                minHeight: searchOpen ? 0 : undefined,
                                maxWidth: "100%",
                            }}
                            panelStyle={PANEL_STYLE}
                        >
                            <Row
                                style={{
                                    gap: 5,
                                    justifyContent: "flex-end",
                                    flexShrink: 0,
                                }}
                            >
                                <button
                                    aria-pressed={searchOpen}
                                    onClick={() => setSearchOpen((s) => !s)}
                                    disabled={!selectedTab}
                                >
                                    Search
                                </button>
                            </Row>
                            {searchOpen && (
                                <SearchPanel
                                    graphId={selectedTab}
                                    rootNode={rootNode}
                                    searchQuery={searchQuery}
                                    onSearchQueryChange={setSearchQuery}
                                    onSelectNode={(nodeId) =>
                                        selectedTab &&
                                        setRootNode(selectedTab, nodeId)
                                    }
                                />
                            )}
                        </ResizablePanel>
                    </Stack>
                </Row>

                {showMcpPopup && mcpFilename !== null && (
                    <McpInstructionsOverlay
                        filename={mcpFilename}
                        onClose={() => setShowMcpPopup(false)}
                    />
                )}
            </Stack>
        </Flamegraph>
    )
}

export default App
