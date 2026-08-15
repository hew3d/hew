/**
 * isolate — Shop Mode's long-press isolate gesture (design §"Gestures"):
 * hide every top-level Object/Instance except the ones reachable from the
 * long-pressed part, renderer-level only (`ViewportApi.setHidden`) — never
 * a kernel/tag mutation, and never marks the document dirty. A persistent
 * "Show all" chip (`setHidden([], [])`) undoes it.
 *
 * `collectLeafIds` (treeModel.ts) already does the "what does this node
 * actually own" expansion — a Group's members recursively, an Object/
 * Instance is its own leaf, a sketch contributes nothing — the SAME
 * expansion the hidden-tag union in App.tsx and the world-bounds panel
 * (objectBounds.ts) both reuse. This module is just that leaf set
 * subtracted from "every object/instance in the document".
 */

import { collectLeafIds, type NodeRef } from '../panels/treeModel'

/** The renderer-level hidden sets `ViewportApi.setHidden` takes. */
export interface IsolateHidden {
  hiddenObjectIds: bigint[]
  hiddenInstanceIds: bigint[]
}

/**
 * Everything OUTSIDE `node`'s leaf set, from the document's full object/
 * instance id lists — i.e. what long-press isolate should hide. Pure
 * function of its inputs; `getGroupMembers` is the caller's
 * `scene.group_members` accessor (matches `collectLeafIds`'s own seam).
 */
export function isolateHiddenFor(
  node: NodeRef,
  allObjectIds: readonly bigint[],
  allInstanceIds: readonly bigint[],
  getGroupMembers: (groupId: bigint) => NodeRef[],
): IsolateHidden {
  const { objectIds, instanceIds } = collectLeafIds(node, getGroupMembers)
  const keepObjects = new Set(objectIds)
  const keepInstances = new Set(instanceIds)
  return {
    hiddenObjectIds: allObjectIds.filter((id) => !keepObjects.has(id)),
    hiddenInstanceIds: allInstanceIds.filter((id) => !keepInstances.has(id)),
  }
}
