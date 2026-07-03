/**
 * Out-of-band store for GraphLike objects (StackGraph and DiffGraph).
 *
 * Graphs can be very large (hundreds of MB of node names, children arrays,
 * and BigInt64Array values). React DevTools recursively introspects every prop
 * on every component during development, which causes severe performance
 * degradation when large objects are passed as props.
 *
 * To avoid this, graphs are never passed as React props. Instead,
 * they are stored here (outside React's awareness), and components receive only
 * a lightweight string ID. Components look up the graph via getGraph() when
 * they actually need it for rendering or computation.
 */
import { type GraphLike } from "./stackGraph"

const store = new Map<string, GraphLike>()
let counter = 0

/** Produces a fresh opaque ID, used when a mutation needs to replace a graph. */
export function generateGraphId(): string {
    return `graph-${counter++}`
}

/** Stores a graph under the given ID. Called when a graph arrives from a worker. */
export function storeGraph(id: string, graph: GraphLike): void {
    store.set(id, graph)
}

/** Retrieves the graph for the given ID, or undefined if not yet loaded. */
export function getGraph(id: string): GraphLike | undefined {
    return store.get(id)
}

/** Removes the graph for the given ID. Called when a tab is deleted or a mutation replaces the graph. */
export function removeGraph(id: string): void {
    store.delete(id)
}
