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
import { translationAffine, affineToFloat64 } from './transformMath'
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
 * path for sketches, which have no single kernel duplicate op. The delta
 * splits the work per target sketch, because a sketch is PLANAR:
 *
 * - **In-plane delta** (stays on the sketch plane) → REPLAY: each source
 *   island's edges are re-drawn into the SAME sketch through the ordinary
 *   sticky machinery (`sketch_add_segment`), translated by `delta`, inside
 *   one drawing gesture per sketch so a single undo removes that sketch's
 *   whole copy. Curve chains replay inside a `sketch_begin_curve_with`
 *   bracket carrying the translated analytic definition, so a copied circle
 *   is a true circle (center snap and all).
 * - **Out-of-plane delta** (leaves the plane — e.g. a ground shape copied up
 *   the Z axis) → NEW-SKETCH copy: a translated replay cannot leave the
 *   plane (points would come off it edge by edge), so a source sketch's
 *   planned islands are copied TOGETHER via `copy_sketch_islands` onto ONE
 *   new sketch on the translated plane, leaving the source untouched. This
 *   reuses the kernel's detach/rebuild machinery — the same
 *   replay-with-identity a MOVE off the plane already takes, minus the
 *   source removal — so the copy keeps curve identity too. Keeping a
 *   sketch's islands together is what preserves a region's HOLES (a hole
 *   boundary is its own island): copying a donut's outer and inner loops
 *   onto separate sketches would silently drop the hole.
 *
 * Granularity mirrors `planSketchTransforms`: a selected edge/curve copies
 * the island it rides with; islands covering every island of their sketch
 * (or a whole-sketch selection) copy the whole sketch. Each in-plane sketch
 * is one undo step; each out-of-plane SOURCE SKETCH is one undo step
 * (regardless of how many of its islands are copied). Islands on different
 * source sketches each land on their own new sketch.
 *
 * FAILURE SEMANTICS — the call is atomic: on any throw the document is left
 * exactly as it was found, including copies earlier iterations had already
 * committed. Getting there takes real work for the in-plane replay, because
 * a sketch replay cannot be rolled back by abandoning it. `sketch_add_segment`
 * mutates the live sketch IMMEDIATELY (in the kernel each add is its own
 * committed clone-validate-swap); the gesture bracket only groups those
 * already-applied edits into one undo step at `sketch_end_gesture`.
 * `sketch_cancel_gesture` is therefore NOT a rollback — it drops the pending
 * undo record and LEAVES the mutations (kernel
 * `Document::cancel_sketch_gesture`: "Any mutations made inside the abandoned
 * bracket stay in the sketch but out of the undo log; cancel-before-mutate is
 * the caller's contract"). Cancelling a replay that had already added an edge
 * would strand a half-copy that Ctrl+Z cannot reach. So recovery runs in two
 * steps:
 *
 * 1. ALWAYS close the bracket with `sketch_end_gesture`, including when the
 *    replay threw — the pattern every other sketch tool follows
 *    (`runSketchGesture`, OffsetTool). It diffs against the pre-gesture
 *    snapshot and records ONE undo step for whatever actually landed, or
 *    nothing at all if the sketch is unchanged. That alone keeps any partial
 *    within reach of a single Ctrl+Z.
 * 2. Then RETRACT those steps, so a refused copy is invisible rather than
 *    merely undoable. Each in-plane gesture's retracting `scene_undo` is
 *    guarded by `history_generation`: the step is only counted when the
 *    generation moved by exactly one across the bracket, which proves the
 *    step is ours and sits on top of the stack. An unguarded undo after a
 *    no-op gesture would pop an unrelated action instead — the
 *    wrong-action-undo bug this repo has already fixed once.
 *
 * The out-of-plane arm needs no such dance: `copy_sketch_islands` is atomic
 * kernel-side (it builds the new sketch fully before mutating and throws with
 * nothing recorded), so a successful call is provably exactly one undo step
 * on top of the stack and is counted directly.
 *
 * Atomicity is also what the caller assumes: MoveTool reselects only the
 * copies it receives as a return value, so a throw has to mean nothing
 * landed — anything else leaves unselected copies sitting in the document
 * behind a toast saying the copy failed.
 *
 * Returns the copies as `sketch-island` NodeRefs (in-plane: post-gesture
 * island ids on the source sketch, merged islands deduped; out-of-plane: the
 * island(s) of each new copy sketch) so the caller can reselect them.
 */
export function duplicateSketchSelection(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  delta: [number, number, number],
): NodeRef[] {
  const { sketches, islands } = planSketchTransforms(wasmScene, selection)

  // Islands to copy, per sketch: whole-sketch selections take every island.
  const islandsBySketch = new Map<bigint, bigint[]>()
  for (const sketch of sketches) {
    islandsBySketch.set(sketch, [...wasmScene.sketch_island_ids(sketch)])
  }
  for (const { sketch, island } of islands) {
    const list = islandsBySketch.get(sketch) ?? []
    list.push(island)
    islandsBySketch.set(sketch, list)
  }

  // Route each target sketch by whether `delta` leaves its plane (kernel
  // PLANE_DIST is 1e-9 m): in-plane sketches replay into themselves; an
  // out-of-plane delta detach-copies each sketch's islands onto ONE new
  // sketch on the translated plane. The out-of-plane path keeps a source
  // sketch's islands TOGETHER (one copy sketch per source, not per island):
  // a region's hole boundary is its own island, so splitting a donut's outer
  // and inner loops onto separate sketches would silently drop the hole.
  const inPlaneEdges = new Map<bigint, bigint[]>()
  const outOfPlaneBySketch = new Map<bigint, bigint[]>()
  for (const [sketch, islandIds] of islandsBySketch) {
    const plane = wasmScene.sketch_plane(sketch)
    if (plane === undefined) continue // stale — skip like any pruned selection
    const off = delta[0] * plane[3] + delta[1] * plane[4] + delta[2] * plane[5]
    if (Math.abs(off) > 1e-9) {
      outOfPlaneBySketch.set(sketch, islandIds)
    } else {
      const all: bigint[] = []
      for (const island of islandIds) {
        all.push(...wasmScene.sketch_island_edges(sketch, island))
      }
      inPlaneEdges.set(sketch, all)
    }
  }

  const committed: NodeRef[] = []
  // Undo steps this call has pushed, newest last — the retraction stack for a
  // mid-copy failure. Only provably-ours steps are counted (see the guards).
  let recorded = 0
  try {
    for (const [sketch, edges] of inPlaneEdges) {
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

      const genBefore = wasmScene.history_generation()
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
      } finally {
        // ALWAYS close the bracket — never `sketch_cancel_gesture`. Every add
        // above has already mutated the live sketch, and `sketch_end_gesture`
        // is what turns whatever landed into ONE undo step (an unchanged
        // sketch records nothing, so this is safe whether the replay finished,
        // partially applied, or threw on its first call). Cancelling would
        // drop the record and keep the geometry.
        wasmScene.sketch_end_gesture(sketch)
        // Exactly +1 proves this gesture recorded exactly one step AND that it
        // is the stack top — nothing else in this loop pushes, every other
        // call being a pure query. On any other delta ownership is unproven,
        // so the step is left for Ctrl+Z rather than risk retracting
        // someone else's.
        if (wasmScene.history_generation() === genBefore + 1n) recorded += 1
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

    // Out-of-plane copies: each SOURCE sketch's islands land TOGETHER on one
    // new sketch on the translated plane via the kernel's detach/rebuild
    // machinery, source left untouched. All of a sketch's islands go in one
    // call so regions (and their holes) re-derive correctly. Every call is
    // atomic and, on success, exactly one undo step on top of the stack —
    // counted directly (no gesture bracket to diff, no history-generation
    // guard needed).
    const affine = affineToFloat64(translationAffine(delta[0], delta[1], delta[2]))
    for (const [sketch, islandIds] of outOfPlaneBySketch) {
      if (islandIds.length === 0) continue
      const copySketch = wasmScene.copy_sketch_islands(
        sketch,
        new BigUint64Array(islandIds),
        affine,
      )
      recorded += 1
      for (const copyIsland of wasmScene.sketch_island_ids(copySketch)) {
        committed.push({ kind: 'sketch-island', id: copyIsland, sketch: copySketch })
      }
    }
  } catch (err) {
    // Retract every step this call recorded, newest first: Move+Alt is ONE
    // user action, so a refused copy puts the document back as it found it.
    try {
      for (let i = 0; i < recorded; i += 1) wasmScene.scene_undo().free()
    } catch {
      // A refused retraction would be a kernel bug (undo is never turned away
      // by a heuristic — DEVELOPMENT.md rule 9). Falling back to "the partial
      // stays, as one undo step per sketch" keeps the floor that matters: no
      // geometry is ever stranded outside the undo log. The replay's own
      // error is the one worth reporting, so it still propagates below.
    }
    throw err
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
