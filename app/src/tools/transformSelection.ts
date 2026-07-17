/**
 * Shared selection commit + preview helpers for the transform tools
 * (Move / Rotate / Scale).
 *
 * Every gesture — one node or many — commits through
 * `Scene.transform_selection`: one kernel call, one undo step, and one
 * session-recording entry regardless of selection size or node kinds.
 *
 * Sketch selections transform as RIGID BODIES at island granularity — the
 * connected shape is the unit, exactly as it is for per-shape Delete:
 *
 * - A `sketch-edge` or `sketch-curve` selection transforms the ISLAND it
 *   belongs to (an open chain of lines is an island too; it moves/rotates
 *   as one rigid body — never silently skipped).
 * - Islands that together cover EVERY island of their sketch fold into one
 *   whole-sketch bake (`transform_sketch`), which keeps every handle stable
 *   and is valid for any axis, including out-of-plane rotations.
 * - A strict subset of a sketch's islands goes through
 *   `transform_sketch_island`, whose kernel now also accepts out-of-plane
 *   transforms by detaching the island into its own sketch.
 */

import * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { nodeKindToNumber, collectLeafIds, nodeRefFromJs } from '../panels/treeModel'
import {
  buildPreviewClone,
  buildMultiPreviewClone,
  buildInstancePreviewClone,
  buildSketchPreviewClone,
} from './transformPreview'

/** The island a sketch-geometry selection transforms as: the node's own
 * island, or the owning island of a selected edge/curve (a drawn curve's
 * NodeRef id is its chain's canonical edge, so the edge→island query covers
 * both). Null for a stale handle — the caller skips it like any other
 * pruned-away selection. */
export function resolveSketchIsland(
  wasmScene: WasmScene,
  node: NodeRef,
): { sketch: bigint; island: bigint } | null {
  if (node.sketch === undefined) return null
  if (node.kind === 'sketch-island') {
    return { sketch: node.sketch, island: node.id }
  }
  if (node.kind === 'sketch-edge' || node.kind === 'sketch-curve') {
    const island = wasmScene.sketch_edge_island(node.sketch, node.id)
    if (island === undefined) return null
    return { sketch: node.sketch, island }
  }
  return null
}

/** How a selection's sketch geometry commits: whole sketches (selected as
 * such, or islands covering the whole sketch) and strict-subset islands. */
export function planSketchTransforms(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
): { sketches: bigint[]; islands: { sketch: bigint; island: bigint }[] } {
  const wholeSketches = new Set<bigint>()
  const islandsBySketch = new Map<bigint, Set<bigint>>()
  for (const node of selection) {
    if (node.kind === 'sketch') {
      wholeSketches.add(node.id)
      continue
    }
    const resolved = resolveSketchIsland(wasmScene, node)
    if (resolved !== null) {
      const set = islandsBySketch.get(resolved.sketch) ?? new Set<bigint>()
      set.add(resolved.island)
      islandsBySketch.set(resolved.sketch, set)
    }
  }
  const islands: { sketch: bigint; island: bigint }[] = []
  for (const [sketch, set] of islandsBySketch) {
    if (wholeSketches.has(sketch)) continue // already moving whole
    // Islands covering EVERY island of their sketch = the whole sketch:
    // fold into one handle-stable whole-sketch bake.
    if (wasmScene.sketch_island_ids(sketch).length === set.size) {
      wholeSketches.add(sketch)
      continue
    }
    for (const island of set) islands.push({ sketch, island })
  }
  return { sketches: [...wholeSketches], islands }
}

/**
 * Commit one affine to the whole selection: sketch geometry per
 * `planSketchTransforms`, everything else via a single
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
  for (const node of selection) {
    if (
      node.kind === 'sketch' ||
      node.kind === 'sketch-island' ||
      node.kind === 'sketch-edge' ||
      node.kind === 'sketch-curve'
    ) {
      continue // handled by the sketch plan below
    }
    kinds.push(nodeKindToNumber(node.kind))
    ids.push(node.id)
  }
  const { sketches, islands } = planSketchTransforms(wasmScene, selection)
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
    // A group's renderable leaves are its world objects AND its instances;
    // `node_leaf_objects` stops at instances (kernel `leaf_objects_under`), so
    // walk the JS tree instead to gather both — otherwise grouped instances
    // are omitted from the drag ghost and freeze in place during the drag.
    const { objectIds, instanceIds } = collectLeafIds(node, (groupId) =>
      wasmScene.group_members(groupId).map(nodeRefFromJs),
    )
    const instanceGroups =
      instanceGroupGetter !== null ? instanceIds.map((id) => instanceGroupGetter(id)) : []
    return buildMultiPreviewClone(objectsGroup, objectIds, instanceGroups)
  }
  if (node.kind === 'instance') {
    const group = instanceGroupGetter !== null ? instanceGroupGetter(node.id) : null
    return buildInstancePreviewClone(group)
  }
  if (node.kind === 'sketch') {
    return buildSketchPreviewClone(wasmScene.sketch_lines(node.id))
  }
  if (
    node.kind === 'sketch-island' ||
    node.kind === 'sketch-edge' ||
    node.kind === 'sketch-curve'
  ) {
    // Sketch geometry transforms at island granularity (the connected
    // shape), so the ghost previews the island a selected edge/curve rides
    // with — matching exactly what the commit will move.
    const resolved = resolveSketchIsland(wasmScene, node)
    if (resolved === null) return null // stale — nothing to preview
    return buildSketchPreviewClone(
      wasmScene.sketch_island_lines(resolved.sketch, resolved.island),
    )
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
