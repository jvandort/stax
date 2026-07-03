import React, { useRef, useState } from "react"
import { Row, Stack } from "./containers"
import { getGraph } from "./graphStore"
import { DiffGraph } from "./stackGraph"

export const GraphPicker: React.FC<{
    tabs: { id: string; name: string }[]
    selectedTab: string | null
    onSelectTab: (id: string) => void
    onDeleteTab: (id: string) => void
    onFileSelected: (file: File) => void
    onDiff: (aTabId: string, bTabId: string) => void
}> = ({
    tabs,
    selectedTab,
    onSelectTab,
    onDeleteTab,
    onFileSelected,
    onDiff,
}) => {
    const fileInputRef = useRef<HTMLInputElement | null>(null)
    const [diffBaseId, setDiffBaseId] = useState<string | null>(null)

    // Diffing a diff makes no sense — diff tabs can be neither the base nor
    // the target of another diff.
    const isDiffTab = (id: string) => getGraph(id) instanceof DiffGraph
    const diffableTabs = tabs.filter(({ id }) => !isDiffTab(id))

    return (
        <Stack style={{ gap: 15 }}>
            <Stack style={{ gap: 5 }}>
                {tabs.map(({ id, name }) => (
                    <Row key={id} style={{ gap: 4 }}>
                        <button
                            style={{
                                textAlign: "left",
                                overflow: "hidden",
                                textOverflow: "ellipsis",
                                flex: 1,
                            }}
                            aria-pressed={selectedTab === id}
                            title={name}
                            onMouseDown={(e) => {
                                if (e.button === 1) {
                                    e.preventDefault()
                                    onDeleteTab(id)
                                }
                            }}
                            onClick={() => onSelectTab(id)}
                        >
                            {name}
                        </button>
                        {diffBaseId === id ? (
                            <button onClick={() => setDiffBaseId(null)}>
                                Cancel
                            </button>
                        ) : diffBaseId !== null ? (
                            <button
                                onClick={() => {
                                    onDiff(diffBaseId, id)
                                    setDiffBaseId(null)
                                }}
                                disabled={isDiffTab(id)}
                            >
                                With
                            </button>
                        ) : (
                            <button
                                onClick={() => setDiffBaseId(id)}
                                disabled={
                                    isDiffTab(id) || diffableTabs.length < 2
                                }
                            >
                                Diff
                            </button>
                        )}
                    </Row>
                ))}
            </Stack>
            <button onClick={() => fileInputRef.current?.click()}>
                Open file...
            </button>
            <input
                type="file"
                style={{ display: "none" }}
                ref={fileInputRef}
                accept=".txt"
                onChange={(e) => {
                    const file = e.target.files?.[0]
                    if (file) {
                        onFileSelected(file)
                    }
                    e.target.value = ""
                }}
            />
        </Stack>
    )
}
