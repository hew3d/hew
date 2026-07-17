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

/**
 * Duplicate a selection's sketch geometry at an offset — Move+Alt's copy
 * path for sketches, which have no kernel duplicate op. The copy is a
 * REPLAY: each source island's edges are re-drawn into the SAME sketch
 * through the ordinary sticky machinery (`sketch_add_segment`), translated
 * by `delta`, inside one drawing gesture per sketch so a single undo
 * removes that sketch's whole copy. Curve chains replay inside a
 * `sketch_begin_curve_with` bracket carrying the translated analytic
 * definition, so a copied circle is a true circle (center snap and all) —
 * the same replay-with-identity approach as the kernel's own
 * `rebuild_island_transformed`, aimed at the source sketch instead of a
 * detach target.
 *
 * Granularity mirrors `planSketchTransforms`: a selected edge/curve copies
 * the island it rides with; islands covering every island of their sketch
 * (or a whole-sketch selection) copy the whole sketch.
 *
 * `delta` must lie in each target sketch's plane — a translated replay
 * cannot leave it (points would come off the plane edge by edge). Checked
 * up front for every planned sketch, before anything mutates; an
 * out-of-plane copy throws with nothing committed. A failure mid-replay
 * cancels that sketch's gesture (snapshot restore), so no sketch is ever
 * left half-copied; earlier sketches' committed gestures stay, exactly like
 * the plain-move loop's semantics.
 *
 * Returns the copies as `sketch-island` NodeRefs (post-gesture island ids,
 * merged islands deduped) so the caller can reselect them.
 */
export function duplicateSketchSelection(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  delta: [number, number, number],
): NodeRef[] {
  const { sketches, islands } = planSketchTransforms(wasmScene, selection)

  // Per-sketch edge sets to replay: whole sketches take every island.
  const edgesBySketch = new Map<bigint, bigint[]>()
  for (const sketch of sketches) {
    const all: bigint[] = []
    for (const island of wasmScene.sketch_island_ids(sketch)) {
      all.push(...wasmScene.sketch_island_edges(sketch, island))
    }
    edgesBySketch.set(sketch, all)
  }
  for (const { sketch, island } of islands) {
    const list = edgesBySketch.get(sketch) ?? []
    list.push(...wasmScene.sketch_island_edges(sketch, island))
    edgesBySketch.set(sketch, list)
  }

  // Validate every target plane BEFORE any mutation: the offset must keep
  // the copy on its sketch plane (kernel PLANE_DIST is 1e-9 m).
  for (const sketch of edgesBySketch.keys()) {
    const plane = wasmScene.sketch_plane(sketch)
    if (plane === undefined) continue // stale — the replay loop will skip it
    const off = delta[0] * plane[3] + delta[1] * plane[4] + delta[2] * plane[5]
    if (Math.abs(off) > 1e-9) {
      throw new Error('PointOffPlane: a sketch copy must stay in its sketch plane')
    }
  }

  const committed: NodeRef[] = []
  for (const [sketch, edges] of edgesBySketch) {
    if (edges.length === 0) continue
    // Snapshot geometry and curve grouping up front — the replay's sticky
    // splits may invalidate source edge handles mid-loop otherwise.
    interface Seg { a: [number, number, number]; b: [number, number, number] }
    const plain: Seg[] = []
    const curves = new Map<string, { geom: number[] | undefined; segs: Seg[] }>()
    for (const edge of edges) {
      const ends = wasmScene.sketch_edge_endpoints(sketch, edge)
      if (ends === undefined) continue // stale handle — nothing to copy
      const seg: Seg = {
        a: [ends[0] + delta[0], ends[1] + delta[1], ends[2] + delta[2]],
        b: [ends[3] + delta[0], ends[4] + delta[1], ends[5] + delta[2]],
      }
      const curve = wasmScene.sketch_edge_curve(sketch, edge)
      if (curve === undefined) {
        plain.push(seg)
      } else {
        const key = curve.toString()
        const entry = curves.get(key) ?? {
          geom: wasmScene.sketch_curve_geom(sketch, curve) as number[] | undefined,
          segs: [],
        }
        entry.segs.push(seg)
        curves.set(key, entry)
      }
    }
    if (plain.length === 0 && curves.size === 0) continue

    wasmScene.sketch_begin_gesture(sketch)
    const newEdges: bigint[] = []
    try {
      const add = (s: Seg): void => {
        const report = wasmScene.sketch_add_segment(
          sketch, s.a[0], s.a[1], s.a[2], s.b[0], s.b[1], s.b[2],
        )
        newEdges.push(...report.new_edges())
        report.free()
      }
      for (const { geom, segs } of curves.values()) {
        if (geom !== undefined) {
          // Translation preserves the circle exactly: center shifts, radius
          // stays — the copy is a true curve, not just facets.
          wasmScene.sketch_begin_curve_with(
            sketch,
            geom[0] + delta[0],
            geom[1] + delta[1],
            geom[2] + delta[2],
            geom[3],
          )
        } else {
          wasmScene.sketch_begin_curve(sketch) // identity-only chain
        }
        for (const s of segs) add(s)
        wasmScene.sketch_end_curve(sketch)
      }
      for (const s of plain) add(s)
      wasmScene.sketch_end_gesture(sketch)
    } catch (err) {
      wasmScene.sketch_cancel_gesture() // snapshot restore — nothing landed
      throw err
    }

    // Map the replayed edges to their (possibly merged) islands.
    const seen = new Set<string>()
    for (const edge of newEdges) {
      const island = wasmScene.sketch_edge_island(sketch, edge)
      if (island === undefined) continue // split away later in the replay
      const key = island.toString()
      if (seen.has(key)) continue
      seen.add(key)
      committed.push({ kind: 'sketch-island', id: island, sketch })
    }
  }
  return committed
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
