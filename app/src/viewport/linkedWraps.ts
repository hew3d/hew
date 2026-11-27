/**
 * Mapping between a linked copy's result and the nodes the app already holds.
 *
 * Copying a loose Object or Group as a linked copy first wraps the original
 * as its own set, so the old Object/Group id stops being a world node and a
 * placement stands in its place. `duplicate_selection_array_linked` reports
 * each wrap; anything that kept the old id (a selection, an expanded Outliner
 * row) goes through `mapSelection` to find the placement instead.
 */

import type { NodeRef } from '../panels/treeModel'
import { nodeRefFromJs } from '../panels/treeModel'

export interface LinkedWrapRef {
  /** The loose node that was wrapped. */
  source: NodeRef
  /** The placement that now stands where `source` was. */
  instance: bigint
  /** The set's shared shape. */
  definition: bigint
}

interface WireNode {
  kind: string
  id: string
}

function parseNode(n: WireNode): NodeRef {
  return nodeRefFromJs({ kind: n.kind, id: BigInt(n.id) })
}

/** Parse `duplicate_selection_array_linked`'s JSON (handles are decimal strings). */
export function parseLinkedResult(json: string): { nodes: NodeRef[]; linked: LinkedWrapRef[] } {
  const raw = JSON.parse(json) as {
    nodes: WireNode[]
    linked: { source: WireNode; instance: string; definition: string }[]
  }
  return {
    nodes: raw.nodes.map(parseNode),
    linked: raw.linked.map((w) => ({
      source: parseNode(w.source),
      instance: BigInt(w.instance),
      definition: BigInt(w.definition),
    })),
  }
}

/** Replace each wrapped source in `sel` with the placement that now stands for it. */
export function mapSelection(sel: readonly NodeRef[], wraps: readonly LinkedWrapRef[]): NodeRef[] {
  if (wraps.length === 0) return [...sel]
  return sel.map((n) => {
    const w = wraps.find((x) => x.source.kind === n.kind && x.source.id === n.id)
    return w === undefined ? n : { kind: 'instance', id: w.instance }
  })
}
