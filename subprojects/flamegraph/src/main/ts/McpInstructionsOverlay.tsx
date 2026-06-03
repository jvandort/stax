import React, { useCallback, useState } from "react"

export const McpInstructionsOverlay = ({
    filename,
    onClose,
}: {
    filename: string
    onClose: () => void
}): React.JSX.Element => {
    const command = `claude mcp remove flamegraph; claude mcp add flamegraph -- node ~/Downloads/${filename}`
    const [copied, setCopied] = useState(false)

    const handleCopy = useCallback(() => {
        void navigator.clipboard.writeText(command).then(() => {
            setCopied(true)
            setTimeout(() => setCopied(false), 2000)
        })
    }, [command])

    return (
        // pointerEvents: "auto" overrides the "none" set on the Flamegraph
        // overlay container that wraps all children.
        <div
            style={{
                position: "fixed",
                inset: 0,
                background: "rgba(0, 0, 0, 0.75)",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                zIndex: 1000,
                pointerEvents: "auto",
            }}
            onClick={onClose}
        >
            <div
                style={{
                    background: "#1e1e1e",
                    border: "1px solid rgba(255, 255, 255, 0.15)",
                    borderRadius: 8,
                    padding: 28,
                    maxWidth: 720,
                    width: "90%",
                    position: "relative",
                    boxShadow: "0 8px 32px rgba(0,0,0,0.6)",
                }}
                onClick={(e) => e.stopPropagation()}
            >
                <button
                    onClick={onClose}
                    style={{
                        position: "absolute",
                        top: 10,
                        right: 10,
                        background: "none",
                        border: "none",
                        color: "rgba(255,255,255,0.6)",
                        fontSize: 20,
                        lineHeight: 1,
                        cursor: "pointer",
                        padding: "2px 6px",
                    }}
                    aria-label="Close"
                >
                    ×
                </button>
                <div
                    style={{
                        marginBottom: 16,
                        fontWeight: "bold",
                        fontSize: 15,
                    }}
                >
                    {filename} downloaded
                </div>
                <div
                    style={{
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "space-between",
                        marginBottom: 8,
                    }}
                >
                    <div style={{ opacity: 0.8 }}>
                        Add the MCP server to Claude Code:
                    </div>
                    <button
                        onClick={handleCopy}
                        style={{
                            background: "rgba(255,255,255,0.08)",
                            border: "1px solid rgba(255,255,255,0.2)",
                            borderRadius: 4,
                            color: "rgba(255,255,255,0.8)",
                            fontSize: 11,
                            cursor: "pointer",
                            padding: "3px 10px",
                            flexShrink: 0,
                        }}
                    >
                        {copied ? "Copied!" : "Copy"}
                    </button>
                </div>
                <pre
                    style={{
                        background: "#0d0d0d",
                        border: "1px solid rgba(255,255,255,0.1)",
                        borderRadius: 4,
                        padding: "10px 14px",
                        margin: "0 0 16px",
                        fontFamily: "monospace",
                        fontSize: 13,
                        overflowX: "auto",
                        whiteSpace: "pre",
                        userSelect: "all",
                    }}
                >
                    {command}
                </pre>
                <div style={{ opacity: 0.8 }}>
                    Start a new Claude Code session to use the server.
                </div>
            </div>
        </div>
    )
}
