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
  translationAffine,
  affineToFloat64,
  applyAffineToPoint,
  applyAffineLinear,
  normalize3,
} from './transformMath'
import {
  buildPreviewClone,
  buildMultiPreviewClone,
  buildInstancePreviewClone,
  buildInstanceMemberPreviewClone,
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
 *
 * `activeInstance`, when set (component-edit-parity.md phase A2 — editing
 * INSIDE a component instance's own definition), routes every selected
 * 'object' node through `transform_def_member(activeInstance, id, affine)`
 * instead — `affineF64` is still the WORLD gesture affine; the kernel
 * conjugates it through the instance's pose itself (never ambiguous under
 * any pose, unlike a scalar distance — no non-uniform-scale refusal here).
 * A definition has no nested groups/instances/sketches of its own (the data
 * model is flat), so this is the only node kind an instance-scoped selection
 * can ever contain; anything else in the selection is skipped rather than
 * routed through the world path, which would silently touch the wrong scene.
 */
export function commitSelectionTransform(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  affineF64: Float64Array,
  activeInstance?: bigint | null,
): void {
  if (activeInstance != null) {
    const { sketches, islands } = planSketchTransforms(wasmScene, selection)
    const objects = selection.filter((node) => node.kind === 'object').map((node) => node.id)
    wasmScene.transform_def_selection(
      activeInstance,
      new BigUint64Array(objects),
      new BigUint64Array(sketches),
      new BigUint64Array(islands.map(({ sketch }) => sketch)),
      new BigUint64Array(islands.map(({ island }) => island)),
      affineF64,
    )
    return
  }
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

/**
 * Half the kernel's `sketch_add_segment` plane tolerance
 * ([`tol::PLANE_DIST`](../../../crates/kernel/src/tol.rs), 1e-9 m) — the
 * margin a rotated point's TRUE distance to the source sketch plane must
 * stay under before `duplicateSketchSelectionByAffine` trusts a same-sketch
 * replay for it. HALF, not the full kernel tolerance, so the exact-projection
 * step it performs next (`projectOntoSketchPlane`, which moves a point by at
 * most its own measured offset — bounded by this margin) lands the fed point
 * provably inside the kernel's own gate, never merely hoping to.
 */
const SAME_SKETCH_PLANE_MARGIN = 0.5e-9

/**
 * Cheap analytic pre-check: does an affine flip a sketch plane's
 * orientation? NOT the in-plane/out-of-plane decision itself (see
 * `duplicateSketchSelectionByAffine`'s doc comment for why that has to be
 * DATA-DRIVEN) — this catches exactly the one case no per-point distance
 * measurement can: a 180°-about-an-in-plane-axis rotation lands every real
 * point of the island back on the plane exactly (every per-point distance
 * check would say "in-plane"), while reversing the region's winding — a
 * mirrored 2D shape embedded in a 3D rotation. Replaying that into the
 * source sketch would land a reflected region regardless of how precisely
 * its points measure, so this gate runs first and unconditionally routes a
 * flip to the copy arm.
 *
 * Returns true (unsafe — route to copy) when the transformed normal has
 * flipped past perpendicular, or when the affine's linear part is
 * degenerate (a zero-length transformed normal has no orientation to trust).
 * A value near zero (a genuine ~90° tilt, not a flip) is deliberately let
 * through here — the DATA-DRIVEN per-point check below rejects it anyway
 * once real geometry is that far off the plane, on its own more precise
 * grounds.
 */
function affineFlipsOrientation(
  affineF64: Float64Array,
  plane: Float64Array,
): boolean {
  const normal: [number, number, number] = [plane[3], plane[4], plane[5]]
  const transformedNormal = normalize3(applyAffineLinear(affineF64, normal))
  if (transformedNormal === null) return true
  const dot =
    transformedNormal[0] * normal[0] +
    transformedNormal[1] * normal[1] +
    transformedNormal[2] * normal[2]
  return dot <= 0
}

/** Signed distance from a point to a sketch plane, in the `[px, py, pz, nx,
 * ny, nz]` form returned by `wasmScene.sketch_plane`. */
function planeSignedDistance(
  plane: Float64Array,
  p: readonly [number, number, number],
): number {
  return (
    (p[0] - plane[0]) * plane[3] +
    (p[1] - plane[1]) * plane[4] +
    (p[2] - plane[2]) * plane[5]
  )
}

/**
 * Project a point exactly onto a sketch plane along its normal. Applied
 * right before a same-sketch replay feeds a point to the kernel: a point
 * already measured within `SAME_SKETCH_PLANE_MARGIN` gets moved by at most
 * that same margin here, landing at TRUE zero plane-distance — the kernel's
 * own (tighter) `PLANE_DIST` gate in `sketch_add_segment` /
 * `sketch_begin_curve_with` can then never reject it, rather than merely
 * being unlikely to.
 */
function projectOntoSketchPlane(
  plane: Float64Array,
  p: readonly [number, number, number],
): [number, number, number] {
  const d = planeSignedDistance(plane, p)
  return [p[0] - d * plane[3], p[1] - d * plane[4], p[2] - d * plane[5]]
}

/**
 * Duplicate a selection's sketch geometry under a general AFFINE (not just a
 * translation) — Rotate+Alt's copy path for sketches. Splits per TARGET
 * sketch, same shape as `duplicateSketchSelection`'s translate arm, but
 * routed DATA-DRIVEN rather than by an analytic classification of the
 * affine alone:
 *
 * A purely analytic test (comparing the affine's transformed plane-point and
 * normal against the source plane, at some cosine tolerance) sounds
 * equivalent to checking real geometry, but isn't: `sketch_add_segment`
 * validates each point it's given against the kernel's actual
 * [`tol::PLANE_DIST`](../../../crates/kernel/src/tol.rs) (1e-9 m), and that
 * gate cares about a real point's distance from the plane, not the angle
 * between two normals. A cosine tolerance loose enough to admit a plausible
 * floating-point rotation (say, an angle within ~4.5e-5 rad of exactly
 * normal) silently admits an axis tilt that displaces any point more than
 * ~22 microns from the pivot/axis well past PLANE_DIST — routing a
 * legitimate-looking in-plane copy (spokes/petals/bolt-ring geometry is
 * routinely meters from its pivot) into a same-sketch replay that the
 * kernel then throws on, when the out-of-plane arm would have succeeded.
 * So this function measures the thing that actually matters instead of
 * predicting it:
 *
 * 1. **Orientation gate** (`affineFlipsOrientation`, analytic, cheap): a
 *    180°-about-an-in-plane-axis rotation lands every real point of an
 *    island exactly back on the plane while reversing its winding — no
 *    per-point distance check can see that, so it's still checked
 *    analytically, first, and unconditionally routes a flip to the copy arm.
 * 2. **Per-point measurement** (data-driven): for every sketch that survives
 *    the gate, each island's actual segment endpoints AND curve centers are
 *    carried through the full affine and their TRUE distance to the source
 *    plane is measured. Only if every one of them lands within
 *    `SAME_SKETCH_PLANE_MARGIN` (half the kernel's own tolerance) does the
 *    sketch route to the same-sketch replay; that replay then projects each
 *    point EXACTLY onto the plane (`projectOntoSketchPlane`) before handing
 *    it to the kernel, so a point already proven to be within half the
 *    tolerance can never trip the kernel's own (tighter) gate. Any sketch
 *    whose real geometry falls outside the margin — no matter how innocuous
 *    the affine looked — routes to the copy arm instead, exactly like a
 *    genuinely out-of-plane transform.
 *
 * The two outcomes:
 *
 * - **In margin** (in-plane rotation: an axis normal to the sketch plane,
 *   pivot anywhere on it — spokes/petals/bolt-ring) → REPLAY into the SAME
 *   sketch, exactly like the translate arm's in-plane branch, except every
 *   point (segment endpoint, curve center) is carried through the FULL
 *   affine (`applyAffineToPoint`) instead of a plain offset, then projected
 *   exactly onto the plane. A rotated circle keeps its radius (the affine is
 *   a rigid rotation, so length is preserved) with a rotated, plane-exact
 *   center; an arc keeps its angular range for free, because the actual
 *   edges are replayed at their rotated positions rather than recomputed
 *   from an angle.
 * - **Out of margin, or orientation-flipping** → copies via
 *   `copy_sketch_islands` onto ONE new sketch with the affine baked in, the
 *   same kernel primitive and same "keep a source sketch's islands
 *   together" reasoning as the translate arm's out-of-plane branch (a
 *   region's hole boundary is its own island). This never throws where the
 *   in-plane arm would have succeeded, because it's the arm with no
 *   PLANE_DIST gate to trip in the first place.
 *
 * Granularity mirrors `planSketchTransforms`: a selected edge/curve copies
 * the island it rides with; islands covering every island of their sketch
 * (or a whole-sketch selection) copy the whole sketch. Atomicity mirrors
 * `duplicateSketchSelection`: every step this call records (in-plane
 * gesture brackets guarded by history-generation, like the translate arm;
 * out-of-plane `copy_sketch_islands` calls counted unconditionally, since
 * each is atomic kernel-side) is retracted, newest first, on any throw — a
 * refused copy puts the document back exactly as it found it, whether the
 * failure came from an in-plane replay or an out-of-plane detach-copy.
 *
 * Returns the copies as `sketch-island` NodeRefs so the caller can reselect
 * them, matching `duplicateSketchSelection`'s return shape.
 */
export function duplicateSketchSelectionByAffine(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  affineF64: Float64Array,
): NodeRef[] {
  return duplicateSketchSelectionByAffineArray(wasmScene, selection, [affineF64])
}

/**
 * `duplicateSketchSelectionByAffine`, generalized to an ORDERED array of
 * cumulative affines — one per copy — for Rotate+Alt's ×N / ÷N array
 * refinement (tool-parity design §1) applied to sketch sources: `affines`
 * is `[stepAffine, 2·stepAffine, …, N·stepAffine]` (as full affines, already
 * composed by the caller — e.g. `rotateAboutPivotAxis` at θ, 2θ, …, Nθ about
 * the same pivot/axis), so this never needs to know the array is a rotation
 * specifically. A single-affine call (`duplicateSketchSelectionByAffine`) is
 * just the `[affineF64]` case.
 *
 * Per-sketch routing (in-plane replay vs. out-of-plane copy — see the
 * ORIGINAL single-affine doc below, unchanged) is decided ONCE for the whole
 * array, not per rep: every rep shares the same rotation axis, so if the
 * first rep's orientation gate or per-point plane margin would route it
 * in-plane, every other rep of the same family does too (a bigger multiple
 * of the same in-plane rotation is still in-plane); if the array's per-point
 * measurement finds ANY rep out of margin, the WHOLE sketch (every rep)
 * routes to the copy arm — never a split where some reps replay same-sketch
 * and others spawn new sketches, which would leave the array's copies
 * scattered across two different kinds of results.
 *
 * In-plane reps replay into the SAME sketch inside ONE
 * `sketch_begin_gesture` / `sketch_end_gesture` bracket covering every rep
 * — so N reps still cost exactly ONE document history entry, matching the
 * object array's `duplicate_selection_array(count=N)` shape (`_resolveArray`
 * in RotateTool.ts relies on this to retract the whole array with a known,
 * fixed number of `scene_undo` calls). Out-of-plane reps have no such
 * kernel primitive to batch through — each `copy_sketch_islands` call is
 * its own atomic step, so N out-of-plane reps cost N history entries; the
 * caller's `recordedEntries` bookkeeping (a history-generation delta, not a
 * hardcoded count) accounts for however many that turns out to be.
 *
 * The original single-affine doc comment, otherwise unchanged:
 *
 * Duplicate a selection's sketch geometry under a general AFFINE (not just a
 * translation) — Rotate+Alt's copy path for sketches. Splits per TARGET
 * sketch, same shape as `duplicateSketchSelection`'s translate arm, but
 * routed DATA-DRIVEN rather than by an analytic classification of the
 * affine alone:
 *
 * A purely analytic test (comparing the affine's transformed plane-point and
 * normal against the source plane, at some cosine tolerance) sounds
 * equivalent to checking real geometry, but isn't: `sketch_add_segment`
 * validates each point it's given against the kernel's actual
 * [`tol::PLANE_DIST`](../../../crates/kernel/src/tol.rs) (1e-9 m), and that
 * gate cares about a real point's distance from the plane, not the angle
 * between two normals. A cosine tolerance loose enough to admit a plausible
 * floating-point rotation (say, an angle within ~4.5e-5 rad of exactly
 * normal) silently admits an axis tilt that displaces any point more than
 * ~22 microns from the pivot/axis well past PLANE_DIST — routing a
 * legitimate-looking in-plane copy (spokes/petals/bolt-ring geometry is
 * routinely meters from its pivot) into a same-sketch replay that the
 * kernel then throws on, when the out-of-plane arm would have succeeded.
 * So this function measures the thing that actually matters instead of
 * predicting it:
 *
 * 1. **Orientation gate** (`affineFlipsOrientation`, analytic, cheap): a
 *    180°-about-an-in-plane-axis rotation lands every real point of an
 *    island exactly back on the plane while reversing its winding — no
 *    per-point distance check can see that, so it's still checked
 *    analytically, first, and unconditionally routes a flip to the copy arm.
 * 2. **Per-point measurement** (data-driven): for every sketch that survives
 *    the gate, each island's actual segment endpoints AND curve centers are
 *    carried through the full affine and their TRUE distance to the source
 *    plane is measured. Only if every one of them lands within
 *    `SAME_SKETCH_PLANE_MARGIN` (half the kernel's own tolerance) does the
 *    sketch route to the same-sketch replay; that replay then projects each
 *    point EXACTLY onto the plane (`projectOntoSketchPlane`) before handing
 *    it to the kernel, so a point already proven to be within half the
 *    tolerance can never trip the kernel's own (tighter) gate. Any sketch
 *    whose real geometry falls outside the margin — no matter how innocuous
 *    the affine looked — routes to the copy arm instead, exactly like a
 *    genuinely out-of-plane transform.
 *
 * The two outcomes:
 *
 * - **In margin** (in-plane rotation: an axis normal to the sketch plane,
 *   pivot anywhere on it — spokes/petals/bolt-ring) → REPLAY into the SAME
 *   sketch, exactly like the translate arm's in-plane branch, except every
 *   point (segment endpoint, curve center) is carried through the FULL
 *   affine (`applyAffineToPoint`) instead of a plain offset, then projected
 *   exactly onto the plane. A rotated circle keeps its radius (the affine is
 *   a rigid rotation, so length is preserved) with a rotated, plane-exact
 *   center; an arc keeps its angular range for free, because the actual
 *   edges are replayed at their rotated positions rather than recomputed
 *   from an angle.
 * - **Out of margin, or orientation-flipping** → copies via
 *   `copy_sketch_islands` onto ONE new sketch with the affine baked in, the
 *   same kernel primitive and same "keep a source sketch's islands
 *   together" reasoning as the translate arm's out-of-plane branch (a
 *   region's hole boundary is its own island). This never throws where the
 *   in-plane arm would have succeeded, because it's the arm with no
 *   PLANE_DIST gate to trip in the first place.
 *
 * Granularity mirrors `planSketchTransforms`: a selected edge/curve copies
 * the island it rides with; islands covering every island of their sketch
 * (or a whole-sketch selection) copy the whole sketch. Atomicity mirrors
 * `duplicateSketchSelection`: every step this call records (in-plane
 * gesture brackets guarded by history-generation, like the translate arm;
 * out-of-plane `copy_sketch_islands` calls counted unconditionally, since
 * each is atomic kernel-side) is retracted, newest first, on any throw — a
 * refused copy puts the document back exactly as it found it, whether the
 * failure came from an in-plane replay or an out-of-plane detach-copy.
 *
 * Returns the copies as `sketch-island` NodeRefs so the caller can reselect
 * them, matching `duplicateSketchSelection`'s return shape.
 */
export function duplicateSketchSelectionByAffineArray(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  affines: readonly Float64Array[],
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

  interface Seg { a: [number, number, number]; b: [number, number, number] }
  interface CurvePlan { center: [number, number, number] | undefined; radius: number | undefined; segs: Seg[] }
  interface RepPlan { plain: Seg[]; curves: Map<string, CurvePlan> }

  // Route each target sketch DATA-DRIVEN (see the doc comment above): the
  // orientation gate runs first and unconditionally sends a flip to the copy
  // arm; everything else gathers its islands' real edges, transforms every
  // endpoint AND curve center through EVERY rep's affine, and measures their
  // TRUE distance to the source plane before deciding — ANY rep out of
  // margin routes the WHOLE sketch (every rep) to the copy arm.
  const inPlanePlan = new Map<bigint, RepPlan[]>()
  const copyBySketch = new Map<bigint, bigint[]>()
  for (const [sketch, islandIds] of islandsBySketch) {
    const plane = wasmScene.sketch_plane(sketch)
    if (plane === undefined) continue // stale — skip like any pruned selection

    const edgeIds: bigint[] = []
    for (const island of islandIds) edgeIds.push(...wasmScene.sketch_island_edges(sketch, island))

    let flips = false
    let withinMargin = true
    const repPlans: RepPlan[] = []
    for (const affineF64 of affines) {
      if (affineFlipsOrientation(affineF64, plane)) {
        flips = true
        break
      }

      const plain: Seg[] = []
      const curves = new Map<string, CurvePlan>()
      for (const edge of edgeIds) {
        const ends = wasmScene.sketch_edge_endpoints(sketch, edge)
        if (ends === undefined) continue // stale handle — nothing to copy
        const a = applyAffineToPoint(affineF64, [ends[0], ends[1], ends[2]])
        const b = applyAffineToPoint(affineF64, [ends[3], ends[4], ends[5]])
        if (
          Math.abs(planeSignedDistance(plane, a)) > SAME_SKETCH_PLANE_MARGIN ||
          Math.abs(planeSignedDistance(plane, b)) > SAME_SKETCH_PLANE_MARGIN
        ) {
          withinMargin = false
        }
        const seg: Seg = { a, b }
        const curve = wasmScene.sketch_edge_curve(sketch, edge)
        if (curve === undefined) {
          plain.push(seg)
        } else {
          const key = curve.toString()
          let entry = curves.get(key)
          if (entry === undefined) {
            const geom = wasmScene.sketch_curve_geom(sketch, curve) as number[] | undefined
            let center: [number, number, number] | undefined
            if (geom !== undefined) {
              center = applyAffineToPoint(affineF64, [geom[0], geom[1], geom[2]])
              if (Math.abs(planeSignedDistance(plane, center)) > SAME_SKETCH_PLANE_MARGIN) {
                withinMargin = false
              }
            }
            entry = { center, radius: geom?.[3], segs: [] }
            curves.set(key, entry)
          }
          entry.segs.push(seg)
        }
      }
      repPlans.push({ plain, curves })
    }

    if (flips || !withinMargin) {
      copyBySketch.set(sketch, islandIds)
      continue
    }
    if (repPlans.every((r) => r.plain.length === 0 && r.curves.size === 0)) continue // nothing to copy

    inPlanePlan.set(sketch, repPlans)
  }

  const committed: NodeRef[] = []
  // Undo steps this call has pushed, newest last — the retraction stack for
  // a mid-copy failure, shared across both arms so the whole gesture is one
  // atomic action (see the doc comment above).
  let recorded = 0
  try {
    for (const [sketch, repPlans] of inPlanePlan) {
      // Re-fetch: a pure query, and the plane used to classify this sketch
      // above (needed again here to project points exactly onto it).
      const plane = wasmScene.sketch_plane(sketch)
      if (plane === undefined) continue // stale — vanishingly unlikely mid-call, never crash on it

      const genBefore = wasmScene.history_generation()
      wasmScene.sketch_begin_gesture(sketch)
      const newEdges: bigint[] = []
      try {
        const add = (s: Seg): void => {
          // Project exactly onto the plane right before feeding the kernel:
          // `withinMargin` already proved the true point sits inside HALF
          // the kernel's own PLANE_DIST, so this moves it by at most that
          // half-margin — `sketch_add_segment` can never reject on plane
          // distance for a point that passed the check above.
          const pa = projectOntoSketchPlane(plane, s.a)
          const pb = projectOntoSketchPlane(plane, s.b)
          const report = wasmScene.sketch_add_segment(
            sketch, pa[0], pa[1], pa[2], pb[0], pb[1], pb[2],
          )
          newEdges.push(...report.new_edges())
          report.free()
        }
        // Every rep replays into this SAME bracket — see the doc comment:
        // that's what keeps an N-rep array down to ONE history entry.
        for (const { plain, curves } of repPlans) {
          for (const { center, radius, segs } of curves.values()) {
            if (center !== undefined && radius !== undefined) {
              // The affine is a rigid rotation (no scale), so the radius
              // survives unchanged — only the center moves, carried through
              // the FULL affine rather than a plain offset, then projected
              // exactly onto the plane for the same reason as `add` above, so
              // a rotated circle stays a true circle (center snap and all).
              const pc = projectOntoSketchPlane(plane, center)
              wasmScene.sketch_begin_curve_with(sketch, pc[0], pc[1], pc[2], radius)
            } else {
              wasmScene.sketch_begin_curve(sketch) // identity-only chain
            }
            for (const s of segs) add(s)
            wasmScene.sketch_end_curve(sketch)
          }
          for (const s of plain) add(s)
        }
      } finally {
        // ALWAYS close the bracket — never `sketch_cancel_gesture` (see
        // `duplicateSketchSelection`'s identical reasoning: every add above
        // already mutated the live sketch).
        wasmScene.sketch_end_gesture(sketch)
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

    // Out-of-plane (or orientation-flipping, or margin-failing) copies: each
    // source sketch's islands land TOGETHER on one new sketch via the
    // kernel's detach/rebuild machinery, source left untouched — same
    // reasoning as `duplicateSketchSelection`'s out-of-plane branch (a
    // region's hole boundary is its own island). One `copy_sketch_islands`
    // call PER REP (no kernel primitive batches these, unlike the in-plane
    // bracket above), so N reps cost N history entries here — the caller's
    // history-generation delta accounts for it, not a hardcoded count.
    for (const [sketch, islandIds] of copyBySketch) {
      if (islandIds.length === 0) continue
      for (const affineF64 of affines) {
        const copySketch = wasmScene.copy_sketch_islands(
          sketch,
          new BigUint64Array(islandIds),
          affineF64,
        )
        recorded += 1
        for (const copyIsland of wasmScene.sketch_island_ids(copySketch)) {
          committed.push({ kind: 'sketch-island', id: copyIsland, sketch: copySketch })
        }
      }
    }
  } catch (err) {
    // Retract every step this call recorded, newest first: the copy gesture
    // is ONE user action, so a refused copy puts the document back as it
    // found it.
    try {
      for (let i = 0; i < recorded; i += 1) wasmScene.scene_undo().free()
    } catch {
      // A refused retraction would be a kernel bug (undo is never turned
      // away by a heuristic — DEVELOPMENT.md rule 9); see
      // `duplicateSketchSelection`'s identical fallback comment.
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
  activeInstance?: bigint | null,
): THREE.Object3D | null {
  const poseDefinitionPreview = (preview: THREE.Object3D | null): THREE.Object3D | null => {
    if (preview === null || activeInstance == null) return preview
    const pose = wasmScene.instance_pose(activeInstance)
    if (pose === undefined) return null
    const posed = new THREE.Group()
    posed.matrixAutoUpdate = false
    posed.matrix.set(
      pose[0], pose[1], pose[2], pose[3],
      pose[4], pose[5], pose[6], pose[7],
      pose[8], pose[9], pose[10], pose[11],
      0, 0, 0, 1,
    )
    posed.add(preview)
    const wrapper = new THREE.Group()
    wrapper.add(posed)
    return wrapper
  }
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
    return poseDefinitionPreview(buildSketchPreviewClone(wasmScene.sketch_lines(node.id)))
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
    return poseDefinitionPreview(
      buildSketchPreviewClone(
        wasmScene.sketch_island_lines(resolved.sketch, resolved.island),
      ),
    )
  }
  if (activeInstance != null) {
    const group = instanceGroupGetter !== null ? instanceGroupGetter(activeInstance) : null
    return buildInstanceMemberPreviewClone(group, activeInstance, node.id)
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
  activeInstance?: bigint | null,
): THREE.Object3D | null {
  if (selection.length === 1) {
    return buildNodePreview(
      wasmScene,
      objectsGroup,
      instanceGroupGetter,
      selection[0],
      activeInstance,
    )
  }
  const group = new THREE.Group()
  group.name = 'SelectionPreview'
  for (const node of selection) {
    const child = buildNodePreview(
      wasmScene,
      objectsGroup,
      instanceGroupGetter,
      node,
      activeInstance,
    )
    if (child !== null) group.add(child)
  }
  return group.children.length > 0 ? group : null
}
