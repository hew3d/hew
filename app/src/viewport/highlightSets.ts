/**
 * What a set of nodes lights up in the viewport, shared by the selection
 * highlight and the transform tools' hover highlight (the node a click would
 * pick up), so the two can never disagree about what a node looks like when
 * picked.
 *
 * Groups own no geometry: they expand (recursively — groups nest) to their
 * leaf objects and instances. A sketch lights its own lines; a drawn shape,
 * curve, line, or region lights its line work, in world coordinates.
 */

import { collectLeafIds, isDraftKind, type NodeRef } from '../panels/treeModel'

export interface HighlightSets {
  /** Leaf object ids (members of the edited placement when inside one). */
  objects: bigint[]
  /** Whole component instances. */
  instances: bigint[]
  sketches: bigint[]
  /** Flat world segment endpoints of every drawn shape, concatenated. */
  draftSegments: Float32Array
}

export interface HighlightDeps {
  groupMembers(groupId: bigint): NodeRef[]
  /** World segment endpoints of a drawn shape (`geometryOf`). */
  draftGeometry(node: NodeRef): Float32Array
}

export function highlightSetsFor(nodes: readonly NodeRef[], deps: HighlightDeps): HighlightSets {
  const objects: bigint[] = []
  const instances: bigint[] = []
  const sketches: bigint[] = []
  const drafts: Float32Array[] = []
  for (const node of nodes) {
    if (node.kind === 'sketch') {
      sketches.push(node.id)
      continue
    }
    if (isDraftKind(node.kind)) {
      drafts.push(deps.draftGeometry(node))
      continue
    }
    const leaves = collectLeafIds(node, (id) => deps.groupMembers(id))
    objects.push(...leaves.objectIds)
    instances.push(...leaves.instanceIds)
  }
  const draftSegments = new Float32Array(drafts.reduce((n, a) => n + a.length, 0))
  let at = 0
  for (const a of drafts) {
    draftSegments.set(a, at)
    at += a.length
  }
  return { objects, instances, sketches, draftSegments }
}
