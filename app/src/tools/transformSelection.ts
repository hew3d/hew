/**
 * Shared selection commit + preview helpers for the transform tools
 * (Move / Rotate / Scale).
 *
 * Every gesture — one node or many — commits through
 * `Scene.transform_selection`: one kernel call, one undo step, and one
 * session-recording entry regardless of selection size or node kinds.
 */

import * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { nodeKindToNumber } from '../panels/treeModel'
import {
  buildPreviewClone,
  buildMultiPreviewClone,
  buildInstancePreviewClone,
  buildSketchPreviewClone,
} from './transformPreview'

/**
 * Commit one affine to the whole selection via a single
 * `transform_selection` call. Kernel errors propagate to the caller's toast
 * handling.
 */
export function commitSelectionTransform(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  affineF64: Float64Array,
): void {
  const kinds: number[] = []
  const ids: bigint[] = []
  const sketches: bigint[] = []
  const islands: { sketch: bigint; island: bigint }[] = []
  for (const node of selection) {
    if (node.kind === 'sketch-edge' || node.kind === 'sketch-curve') {
      continue // lines/curves are not transformable (v1: select/delete)
    }
    if (node.kind === 'sketch-island' && node.sketch !== undefined) {
      islands.push({ sketch: node.sketch, island: node.id })
      continue
    }
    if (node.kind === 'sketch') {
      sketches.push(node.id)
    } else {
      kinds.push(nodeKindToNumber(node.kind))
      ids.push(node.id)
    }
  }
  // Islands transform one-by-one, but VALIDATE all of them first so one
  // refused landing aborts the whole move before anything commits — no
  // half-moved selections. (Single-threaded: nothing mutates between the
  // validation pass and the commits.)
  for (const { sketch, island } of islands) {
    if (!wasmScene.can_transform_sketch_island(sketch, island, affineF64)) {
      throw new Error('WouldRetopologize: the move would land a shape on other geometry')
    }
  }
  for (const { sketch, island } of islands) {
    wasmScene.transform_sketch_island(sketch, island, affineF64)
  }
  if (kinds.length > 0 || sketches.length > 0) {
    wasmScene.transform_selection(
      new Uint8Array(kinds),
      new BigUint64Array(ids),
      new BigUint64Array(sketches),
      affineF64,
    )
  }
}

/** The ghost preview for one node — the shape all three transform tools share. */
export function buildNodePreview(
  wasmScene: WasmScene,
  objectsGroup: THREE.Group | null,
  instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null,
  node: NodeRef,
): THREE.Object3D | null {
  if (node.kind === 'group') {
    const leafIds = Array.from(wasmScene.node_leaf_objects(1, node.id))
    return buildMultiPreviewClone(objectsGroup, leafIds)
  }
  if (node.kind === 'instance') {
    const group = instanceGroupGetter !== null ? instanceGroupGetter(node.id) : null
    return buildInstancePreviewClone(group)
  }
  if (node.kind === 'sketch') {
    return buildSketchPreviewClone(wasmScene.sketch_lines(node.id))
  }
  if (node.kind === 'sketch-island' && node.sketch !== undefined) {
    return buildSketchPreviewClone(wasmScene.sketch_island_lines(node.sketch, node.id))
  }
  if (node.kind === 'sketch-edge' || node.kind === 'sketch-curve') {
    return null // not transformable — contributes nothing to the ghost
  }
  return buildPreviewClone(objectsGroup, node.id)
}

/**
 * One ghost preview for a whole selection: each node's preview under a shared
 * group so the tool can translate/rotate it as a unit. Null when nothing in
 * the selection has previewable geometry.
 */
export function buildSelectionPreview(
  wasmScene: WasmScene,
  objectsGroup: THREE.Group | null,
  instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null,
  selection: readonly NodeRef[],
): THREE.Object3D | null {
  if (selection.length === 1) {
    return buildNodePreview(wasmScene, objectsGroup, instanceGroupGetter, selection[0])
  }
  const group = new THREE.Group()
  group.name = 'SelectionPreview'
  for (const node of selection) {
    const child = buildNodePreview(wasmScene, objectsGroup, instanceGroupGetter, node)
    if (child !== null) group.add(child)
  }
  return group.children.length > 0 ? group : null
}
