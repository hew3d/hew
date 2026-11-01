/**
 * Shared selection commit + preview helpers for the transform tools
 * (Move / Rotate / Scale).
 *
 * Every gesture — one node or many — commits through
 * `Scene.transform_selection`: one kernel call, one undo step, and one
 * session-recording entry regardless of selection size or node kinds. A
 * gesture that spans several kernel edits (drawing in several planes, drawing
 * plus objects, a copy array) runs inside a history bracket, so it is still ONE
 * undo step and a refusal restores the document exactly.
 *
 * DRAWN things (the `draft-*` refs of planeUnits.ts: lines, curves, shapes
 * on a solid's face or over the void or both at once) go through the
 * drafting engine, one path for every store (unified-drafting design §8):
 *
 * - The selection is grouped by PLANE CONTEXT. A selected edge or curve
 *   moves the shape (island) it belongs to, exactly as a sketch edge always
 *   did — the connected shape is the unit.
 * - Per context, ONE draft: `draft_transform_edges` over the shape's edge
 *   keys (copy mode: `draft_copy_edges`). Landings ARRANGE like a draw — a
 *   shape moved onto others splits and merges with them, half over a face
 *   and half over the void is fine.
 * - A MIXED shape (part of it rides with a selected object, the rest is
 *   free or on unselected objects) is moved by drawing it there: its
 *   non-riding edges are captured and erased, the objects transform, and the
 *   captured edges are drawn again carried through the transform (circles
 *   stay circles under a similarity). Copies draw the captured edges per rep.
 * - A transform that LEAVES the plane commits only for a shape drawn
 *   entirely over the void (every edge in a free sketch): it detaches into
 *   its own sketch exactly as it always did. A shape with any edge on a
 *   solid's face refuses `NotInPlane`: a face shape stays on its face.
 * - After the commit the selection is re-resolved BY GEOMETRY (the moved
 *   edge positions in a fresh listing), so a typed retype or a second move
 *   targets the moved shape.
 *
 * Whole free sketches (`'sketch'` refs) fold into one whole-sketch bake
 * (`transform_sketch`), valid for any axis, as before.
 */

import * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { isDraftKind, nodeKindToNumber, collectLeafIds, nodeRefFromJs } from '../panels/treeModel'
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
  markSessionGhost,
} from './transformPreview'
import {
  allFree,
  ctxOfObjectPlane,
  ctxOfSketch,
  ctxThroughAffine,
  draftRef,
  isDraftRef,
  keysToArray,
  planeOfCtx,
  readPlaneUnits,
  resolveDraft,
  segmentsOf,
  storesOf,
  unitsAtSegments,
  type PlaneEdge,
} from './planeUnits'
import { runDraft } from './draftCommit'
import { applyAffine3x4, invertAffine3x4 } from '../viewport/geoHelpers'
import { t } from '@lingui/core/macro'

type V3 = [number, number, number]

/** The islands of one sketch that move together (one kernel call). */
export interface SketchIslandGroup {
  sketch: bigint
  islands: bigint[]
}

/** The refs of `selection` that are not drawing refs. */
function nonDraft(selection: readonly NodeRef[]): NodeRef[] {
  return selection.filter((n) => !isDraftKind(n.kind))
}

/** How a selection's whole-sketch geometry commits: whole sketches (selected
 * as such, or islands covering the whole sketch) and strict-subset islands
 * (only ever supplied by `freeIslands`, the detaching free shapes). */
export function planSketchTransforms(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  freeIslands: readonly SketchIslandGroup[] = [],
): { sketches: bigint[]; islands: SketchIslandGroup[] } {
  const wholeSketches = new Set<bigint>()
  const islandsBySketch = new Map<bigint, Set<bigint>>()
  for (const node of selection) {
    if (node.kind === 'sketch') wholeSketches.add(node.id)
  }
  for (const g of freeIslands) {
    const set = islandsBySketch.get(g.sketch) ?? new Set<bigint>()
    for (const i of g.islands) set.add(i)
    islandsBySketch.set(g.sketch, set)
  }
  const islands: SketchIslandGroup[] = []
  for (const [sketch, set] of islandsBySketch) {
    if (wholeSketches.has(sketch)) continue // already moving whole
    // Islands covering EVERY island of their sketch = the whole sketch:
    // fold into one handle-stable whole-sketch bake.
    if (wasmScene.sketch_island_ids(sketch).length === set.size) {
      wholeSketches.add(sketch)
      continue
    }
    islands.push({ sketch, islands: [...set] })
  }
  return { sketches: [...wholeSketches], islands }
}

// ── Drawn things (draft refs) ───────────────────────────────────────────────

/** The drawn shapes of one plane context a selection moves together. */
export interface DraftGroup {
  ctx: string
  /** The selection nodes that fed this group. */
  nodes: NodeRef[]
  /** The shapes (islands) the group moves, by unit key. */
  unitKeys: string[]
  /** The edges of those shapes that are transformed as drawing (those riding
   *  with a selected owner are left to the owner's own transform). */
  edges: PlaneEdge[]
  /** True when some of a shape's edges ride with a selected owner: only the
   *  rest is drafted: erased and drawn again around the owners' move. */
  mixed: boolean
}

/** The owners a selection carries drawing along with: objects selected
 *  directly or through a selected group, and component instances likewise. */
export interface RidingOwners {
  objects: Set<bigint>
  instances: Set<bigint>
}

export function ridingOwners(wasmScene: WasmScene, selection: readonly NodeRef[]): RidingOwners {
  const objects = new Set<bigint>()
  const instances = new Set<bigint>()
  for (const n of selection) {
    if (n.kind === 'object') objects.add(n.id)
    else if (n.kind === 'instance') instances.add(n.id)
    else if (n.kind === 'group') {
      const { objectIds, instanceIds } = collectLeafIds(n, (g) => wasmScene.group_members(g).map(nodeRefFromJs))
      for (const o of objectIds) objects.add(o)
      for (const i of instanceIds) instances.add(i)
    }
  }
  return { objects, instances }
}

/** True when `edge` (drawn in plane context `ctx`) moves/copies/deletes WITH
 *  its owner: its context lies inside a selected instance, or it is stored on
 *  a selected object's own faces (free sketch edges never ride an object). */
export function edgeRides(owners: RidingOwners, ctx: string, edge: PlaneEdge): boolean {
  const inst = planeOfCtx(ctx)?.instance ?? null
  if (inst !== null && owners.instances.has(inst)) return true
  return edge.store.kind !== 'free' && owners.objects.has(edge.store.object)
}

/**
 * The selection without the drawn shapes that ride with their own object: a
 * draft ref lying wholly on a selected object's faces (or its owned
 * sketches), or inside a selected component instance (or one inside a
 * selected group), moves/copies WITH that owner, so transforming it as a
 * draft as well would apply the transform twice (or refuse a vertical move
 * as NotInPlane). Ground/free drawing is untouched, and so is a shape with
 * any edge that does not ride (a mixed shape keeps its ref;
 * `planDraftTransforms` then drafts only its non-riding edges). Mirrors
 * Delete's "an object also in this delete covers the drawing on it" rule.
 */
export function dropRidingDrafts(wasmScene: WasmScene, selection: readonly NodeRef[]): NodeRef[] {
  if (!selection.some(isDraftRef)) return [...selection]
  const owners = ridingOwners(wasmScene, selection)
  if (owners.objects.size === 0 && owners.instances.size === 0) return [...selection]
  return selection.filter((node) => {
    if (!isDraftRef(node)) return true
    const inst = planeOfCtx(node.ctx)?.instance ?? null
    if (inst !== null && owners.instances.has(inst)) return false
    const r = resolveDraft(wasmScene, node)
    if (r === null || r.edges.length === 0) return true
    return !r.edges.every((e) => edgeRides(owners, node.ctx, e))
  })
}

/**
 * What Delete must erase through drafts: per plane context, the edge keys of
 * every selected drawn shape that does NOT go with another part of the same
 * delete — edges riding with a deleted object (directly or inside a deleted
 * group) or a deleted component instance, and free edges of a sketch
 * deleted whole. Select All + Delete is thereby one node delete: drawing on a
 * group's faces dies with the group (one undo restores both), and the shared
 * definition of a deleted instance is never stripped first. `huskCandidates`
 * are the free sketches the erase may leave empty.
 */
export function draftEdgesToErase(
  wasmScene: WasmScene,
  nodes: readonly NodeRef[],
  deletedSketches: ReadonlySet<bigint>,
): { draftEdgeKeys: Map<string, Set<string>>; huskCandidates: Set<bigint> } {
  const owners = ridingOwners(wasmScene, nodes)
  const draftEdgeKeys = new Map<string, Set<string>>()
  const huskCandidates = new Set<bigint>()
  for (const n of nodes) {
    if (!isDraftRef(n)) continue
    const r = resolveDraft(wasmScene, n)
    if (r === null) continue // stale — an earlier step consumed it
    const rest = r.edges.filter(
      (e) => !edgeRides(owners, n.ctx, e) && !(e.store.kind === 'free' && deletedSketches.has(e.store.sketch)),
    )
    if (rest.length === 0) continue
    const keys = draftEdgeKeys.get(n.ctx) ?? new Set<string>()
    for (const e of rest) {
      keys.add(e.key)
      if (e.store.kind === 'free') huskCandidates.add(e.store.sketch)
    }
    draftEdgeKeys.set(n.ctx, keys)
  }
  return { draftEdgeKeys, huskCandidates }
}

/** Group the draft refs of `selection` by plane context. Edges and curves
 *  lift to their shape; stale refs are skipped like any pruned selection.
 *  A shape part of which rides with a selected owner (a mixed shape) keeps
 *  only its non-riding edges in the group, flagged `mixed` so the commit
 *  draws it again around the owners' move (erase, move, redraw). */
export function planDraftTransforms(wasmScene: WasmScene, selection: readonly NodeRef[]): DraftGroup[] {
  const groups = new Map<string, DraftGroup>()
  const owners = ridingOwners(wasmScene, selection)
  const anyOwner = owners.objects.size > 0 || owners.instances.size > 0
  for (const node of dropRidingDrafts(wasmScene, selection)) {
    if (!isDraftRef(node)) continue
    const r = resolveDraft(wasmScene, node)
    if (r === null || r.unit === null) continue
    const all = r.unit.edges
    const own = anyOwner ? all.filter((e) => !edgeRides(owners, node.ctx, e)) : all
    if (own.length === 0) continue
    let g = groups.get(node.ctx)
    if (g === undefined) {
      g = { ctx: node.ctx, nodes: [], unitKeys: [], edges: [], mixed: false }
      groups.set(node.ctx, g)
    }
    g.nodes.push(node)
    if (own.length < all.length) g.mixed = true
    if (g.unitKeys.includes(r.unit.key)) continue
    g.unitKeys.push(r.unit.key)
    g.edges.push(...own)
  }
  return [...groups.values()]
}

/** Largest distance any of the group's endpoints (and circle centers) lands
 *  from its plane under `affine` — an in-plane transform keeps this ~0. */
function offPlaneAfter(group: DraftGroup, affine: Float64Array): number {
  const pl = planeOfCtx(group.ctx)
  if (pl === null) return 0
  let worst = 0
  const check = (p: V3) => {
    const q = applyAffineToPoint(affine, p)
    const d = Math.abs(
      (q[0] - pl.point[0]) * pl.normal[0] + (q[1] - pl.point[1]) * pl.normal[1] + (q[2] - pl.point[2]) * pl.normal[2],
    )
    if (d > worst) worst = d
  }
  for (const e of group.edges) {
    check(e.a)
    check(e.b)
    if (e.curve !== null) check(e.curve.center)
  }
  return worst
}

/** Tolerance for "still in the plane" (the kernel's own plane gate is 1e-9). */
const IN_PLANE_EPS = 0.5e-9

function inPlane(group: DraftGroup, affine: Float64Array): boolean {
  return offPlaneAfter(group, affine) <= IN_PLANE_EPS
}

/** The NotInPlane refusal for a face shape asked to leave its face. */
function notInPlane(): Error {
  return new Error("NotInPlane: A shape on a face can only move within the face's plane")
}

// ── One gesture = one history step ──────────────────────────────────────────

/** Scenes with a bracket open right now (brackets do not nest in the kernel,
 *  so a helper called inside another helper's bracket joins it). */
const bracketed = new WeakSet<object>()

/**
 * Run `body` as ONE undo step: every edit it records lands as a single
 * history entry labeled `label`, and ANY throw restores the document exactly
 * as it was (geometry, undo and redo stacks) before rethrowing — no
 * hand-written retraction. `needed = false` runs `body` bare, for a gesture
 * already known to record at most one atomic entry (the bracket snapshots the
 * document, so it is skipped when it cannot matter). A nested call joins the
 * enclosing bracket. The bracket is never left open.
 */
export function inHistoryBracket<T>(wasmScene: WasmScene, label: string, body: () => T, needed = true): T {
  if (!needed || bracketed.has(wasmScene)) return body()
  wasmScene.history_bracket_open()
  bracketed.add(wasmScene)
  let committed = false
  try {
    const result = body()
    wasmScene.history_bracket_commit(label)
    committed = true
    return result
  } finally {
    bracketed.delete(wasmScene)
    if (!committed) {
      try {
        wasmScene.history_bracket_abort()
      } catch {
        // The original error is the one worth surfacing.
      }
    }
  }
}

// ── Drawing a mixed shape "there" ───────────────────────────────────────────

/** One curve run of a captured shape: the facets that stay a circle (or
 *  regular polygon) when they are drawn again. */
interface CapturedRun {
  kind: 'circle' | 'polygon'
  center: V3
  radius: number | null
  edges: PlaneEdge[]
}

/** A group's non-riding edges as they stand now, ready to be drawn again at
 *  a transformed position: per curve run (kind, center, radius, facets) and
 *  the plain remainder. */
interface CapturedDrawing {
  runs: CapturedRun[]
  plain: PlaneEdge[]
}

function captureDrawing(wasmScene: WasmScene, g: DraftGroup): CapturedDrawing {
  const mine = new Map(g.edges.map((e) => [e.key, e]))
  const inRun = new Set<string>()
  const runs: CapturedRun[] = []
  const pu = readPlaneUnits(wasmScene, g.ctx)
  for (const unit of pu?.units ?? []) {
    for (const run of unit.curves) {
      const edges = run.edges.flatMap((k) => {
        const e = mine.get(k)
        return e === undefined ? [] : [e]
      })
      if (edges.length === 0) continue
      for (const e of edges) inRun.add(e.key)
      runs.push({ kind: run.kind, center: run.center, radius: run.radius, edges })
    }
  }
  return { runs, plain: g.edges.filter((e) => !inRun.has(e.key)) }
}

/** The uniform scale `affine` applies within `ctx`'s plane, or `null` when it
 *  is not a similarity there (a circle drawn through it is no longer one). */
function planeSimilarity(ctx: string, affine: Float64Array): number | null {
  const pl = planeOfCtx(ctx)
  if (pl === null) return null
  const n = pl.normal
  const axis: V3 =
    Math.abs(n[0]) <= Math.abs(n[1]) && Math.abs(n[0]) <= Math.abs(n[2]) ? [1, 0, 0] : Math.abs(n[1]) <= Math.abs(n[2]) ? [0, 1, 0] : [0, 0, 1]
  const u = normalize3([n[1] * axis[2] - n[2] * axis[1], n[2] * axis[0] - n[0] * axis[2], n[0] * axis[1] - n[1] * axis[0]])
  if (u === null) return null
  const v: V3 = [n[1] * u[2] - n[2] * u[1], n[2] * u[0] - n[0] * u[2], n[0] * u[1] - n[1] * u[0]]
  const lu = applyAffineLinear(affine, u)
  const lv = applyAffineLinear(affine, v)
  const su = Math.hypot(lu[0], lu[1], lu[2])
  const sv = Math.hypot(lv[0], lv[1], lv[2])
  if (!(su > 0) || Math.abs(su - sv) > 1e-9 * su) return null
  const dot = lu[0] * lv[0] + lu[1] * lv[1] + lu[2] * lv[2]
  return Math.abs(dot) <= 1e-9 * su * sv ? su : null
}

/** Draw `cap` carried through `affine` into the open draft `d` (plane
 *  context `ctx`): each curve run re-bracketed — a circle or polygon keeps
 *  its analytic circle (center mapped, radius scaled) under a similarity,
 *  anything else becomes a plain chain — then the plain segments. */
function drawCaptured(wasmScene: WasmScene, d: bigint, ctx: string, cap: CapturedDrawing, affine: Float64Array): void {
  const scale = planeSimilarity(ctx, affine)
  const add = (e: PlaneEdge) => {
    const a = applyAffineToPoint(affine, e.a)
    const b = applyAffineToPoint(affine, e.b)
    wasmScene.draft_add_segment(d, a[0], a[1], a[2], b[0], b[1], b[2]).free()
  }
  for (const run of cap.runs) {
    if (scale !== null && run.radius !== null) {
      const c = new Float64Array(applyAffineToPoint(affine, run.center))
      if (run.kind === 'polygon') wasmScene.draft_begin_polygon_with(d, c, run.radius * scale)
      else wasmScene.draft_begin_curve_with(d, c, run.radius * scale)
    } else {
      wasmScene.draft_begin_curve(d)
    }
    for (const e of run.edges) add(e)
    wasmScene.draft_end_curve(d)
  }
  for (const e of cap.plain) add(e)
}

/** Draw a captured shape at each of `affines` in ONE draft of `g`'s plane. */
function redrawCaptured(
  wasmScene: WasmScene,
  g: DraftGroup,
  cap: CapturedDrawing,
  affines: readonly Float64Array[],
  label: string,
): void {
  runDraft(wasmScene, { kind: 'ctx', ctx: g.ctx }, label, (d) => {
    for (const affine of affines) drawCaptured(wasmScene, d, g.ctx, cap, affine)
  })
}

/** The free islands a pure-void group detaches, per store sketch, found by
 *  the edges' own positions. The listing's endpoints are WORLD coordinates;
 *  a definition sketch inside an entered instance lives in the definition's
 *  own frame, so the endpoints go through the inverse instance pose first.
 *  An island that cannot be found refuses — never a silent no-op. */
function freeIslandsOf(wasmScene: WasmScene, group: DraftGroup): SketchIslandGroup[] {
  const instance = planeOfCtx(group.ctx)?.instance ?? null
  let toLocal: ((p: V3) => V3) | null = null
  if (instance !== null) {
    const pose = wasmScene.instance_pose(instance)
    const inv = pose === undefined ? null : invertAffine3x4(pose)
    if (inv === null) {
      throw new Error('StaleReference: the component instance holding this shape is gone')
    }
    toLocal = (p) => applyAffine3x4(inv, p)
  }
  const bySketch = new Map<bigint, Set<bigint>>()
  for (const e of group.edges) {
    if (e.store.kind !== 'free') continue
    const a = toLocal === null ? e.a : toLocal(e.a)
    const b = toLocal === null ? e.b : toLocal(e.b)
    const island = wasmScene.sketch_island_at_edge_positions(e.store.sketch, new Float64Array(a), new Float64Array(b))
    if (island === undefined) {
      throw new Error('StaleReference: could not find the shape to move out of its plane; nothing was changed')
    }
    const set = bySketch.get(e.store.sketch) ?? new Set<bigint>()
    set.add(island)
    bySketch.set(e.store.sketch, set)
  }
  return [...bySketch].map(([sketch, set]) => ({ sketch, islands: [...set] }))
}

/** Segment pairs of the group's edges carried through `affine` — where the
 *  moved shapes will be found again. */
function movedAnchors(group: DraftGroup, affine: Float64Array): [V3, V3][] {
  return group.edges.map((e) => [applyAffineToPoint(affine, e.a), applyAffineToPoint(affine, e.b)] as [V3, V3])
}

/** The island refs holding `anchors` in the listing of `ctx` (fresh). */
function refsAtAnchors(wasmScene: WasmScene, ctx: string | null, anchors: readonly [V3, V3][]): NodeRef[] {
  if (ctx === null) return []
  const pu = readPlaneUnits(wasmScene, ctx)
  if (pu === null) return []
  return unitsAtSegments(pu, anchors).map((u) => draftRef('draft-island', ctx, u.key))
}

/** Replace each group's nodes in `committed` with the refs found at its
 *  moved anchors (in the position of the group's first node), dropping a
 *  group that cannot be re-found. Duplicate refs collapse. */
function repointGroups(
  committed: NodeRef[],
  groups: readonly DraftGroup[],
  found: ReadonlyMap<DraftGroup, NodeRef[]>,
): void {
  const seen = new Set<string>()
  const out: NodeRef[] = []
  const placed = new Set<DraftGroup>()
  for (const node of committed) {
    const g = groups.find((x) => x.nodes.includes(node))
    if (g === undefined) {
      out.push(node)
      continue
    }
    if (placed.has(g)) continue
    placed.add(g)
    for (const ref of found.get(g) ?? []) {
      const k = `${ref.ctx}:${ref.key}`
      if (seen.has(k)) continue
      seen.add(k)
      out.push(ref)
    }
  }
  committed.splice(0, committed.length, ...out)
}

/**
 * Commit one affine to the whole selection: drawn shapes per plane context
 * through drafts, whole sketches and everything else via a single
 * `transform_selection` call. Kernel errors propagate to the caller's toast
 * handling.
 *
 * The gesture is ONE undo step labeled `label`, and atomic: when it would
 * record more than one history entry (drafts in several contexts, drafts and
 * objects, objects and whole sketches…) it runs inside a history bracket, and
 * any refusal restores the document exactly (`inHistoryBracket`).
 *
 * A shape drawn half on a selected object's face and half beside it (MIXED)
 * is moved by drawing it there, in a fixed order that needs no guessing:
 * (a) its non-riding edges are captured from the listing and erased; (b) the
 * objects transform — their face parts ride along; (c) the captured edges are
 * drawn again, carried through the affine, each curve run re-bracketed so a
 * circle stays a circle under a similarity. A mixed shape that leaves its
 * plane (every non-riding edge free) detaches through the sketch machinery
 * after the owners move.
 *
 * `activeInstance`, when set (component-edit-parity.md phase A2 — editing
 * INSIDE a component instance's own definition), routes every selected
 * 'object' node through `transform_def_member(activeInstance, id, affine)`
 * instead — `affineF64` is still the WORLD gesture affine; the kernel
 * conjugates it through the instance's pose itself (never ambiguous under
 * any pose, unlike a scalar distance — no non-uniform-scale refusal here).
 * Drawn shapes inside an instance carry the instance in their context, so
 * their drafts conjugate the same way.
 */
export function commitSelectionTransform(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  affineF64: Float64Array,
  activeInstance?: bigint | null,
  label = t({ context: 'tool', message: 'Move' }),
): NodeRef[] {
  const committed: NodeRef[] = [...selection]
  const groups = planDraftTransforms(wasmScene, selection)
  // Classify and validate EVERY group before anything lands: in-plane groups
  // draft (a mixed one is captured to be drawn again); a pure-void group that
  // leaves its plane detaches; a group with any face edge that leaves its
  // plane refuses, whole selection.
  const pureDrafts: DraftGroup[] = []
  const redraws = new Map<DraftGroup, CapturedDrawing>()
  const detachIslands: SketchIslandGroup[] = []
  const detachPostIslands: SketchIslandGroup[] = []
  const detaching: DraftGroup[] = []
  for (const g of groups) {
    if (inPlane(g, affineF64)) {
      if (g.mixed) redraws.set(g, captureDrawing(wasmScene, g))
      else pureDrafts.push(g)
    } else if (g.edges.every((e) => e.store.kind === 'free')) {
      detaching.push(g)
      ;(g.mixed ? detachPostIslands : detachIslands).push(...freeIslandsOf(wasmScene, g))
    } else {
      throw notInPlane()
    }
  }
  const { sketches, islands } = planSketchTransforms(wasmScene, selection, detachIslands)
  // The mixed islands' own plan (folded to whole sketches like any other),
  // minus anything the main plan already moves.
  let post: { sketches: bigint[]; islands: SketchIslandGroup[] } = { sketches: [], islands: [] }
  if (detachPostIslands.length > 0) {
    const plan = planSketchTransforms(wasmScene, [], detachPostIslands)
    const moving = new Set<bigint>([...sketches, ...islands.map((g) => g.sketch)])
    post = {
      sketches: plan.sketches.filter((x) => !moving.has(x)),
      islands: plan.islands.filter((g) => !moving.has(g.sketch)),
    }
  }
  const kinds: number[] = []
  const ids: bigint[] = []
  for (const node of nonDraft(selection)) {
    if (node.kind === 'sketch') continue // handled by the sketch plan
    if (activeInstance != null && node.kind !== 'object') continue
    kinds.push(nodeKindToNumber(node.kind))
    ids.push(node.id)
  }
  // History entries this gesture would record unbracketed.
  const hasPost = post.sketches.length > 0 || post.islands.length > 0
  const mainSteps =
    activeInstance != null
      ? (ids.length > 0 || sketches.length > 0 || islands.length > 0 ? 1 : 0) + (hasPost ? 1 : 0)
      : islands.length + (kinds.length > 0 || sketches.length > 0 ? 1 : 0) + post.islands.length + (post.sketches.length > 0 ? 1 : 0)
  const steps = pureDrafts.length + 2 * redraws.size + mainSteps

  const found = new Map<DraftGroup, NodeRef[]>()
  // Shapes riding with a selected owner (dropped from the plan) move with
  // it; remember where they are so they stay selected where they land.
  const planned = new Set(groups.flatMap((g) => g.nodes))
  const riding = selection
    .filter((n) => isDraftRef(n) && !planned.has(n))
    .map((n) => ({ node: n, segs: segmentsOf(wasmScene, n) }))
  const ridingFound = new Map<NodeRef, NodeRef[]>()
  inHistoryBracket(
    wasmScene,
    label,
    () => {
      // (a) Erase the mixed shapes' non-riding edges; the pure shapes move.
      for (const g of redraws.keys()) {
        runDraft(wasmScene, { kind: 'ctx', ctx: g.ctx }, label, (d) => {
          wasmScene.draft_remove_edges(d, keysToArray(g.edges.map((e) => e.key)))
        })
      }
      for (const g of pureDrafts) {
        runDraft(wasmScene, { kind: 'ctx', ctx: g.ctx }, label, (d) => {
          wasmScene.draft_transform_edges(d, keysToArray(g.edges.map((e) => e.key)), affineF64)
        })
      }
      // (b) The owners (and detaching shapes) transform.
      if (activeInstance != null) {
        const defMove = (objs: bigint[], sks: bigint[], isl: SketchIslandGroup[]) => {
          wasmScene.transform_def_selection(
            activeInstance,
            new BigUint64Array(objs),
            new BigUint64Array(sks),
            new BigUint64Array(isl.flatMap(({ sketch, islands: list }) => list.map(() => sketch))),
            new BigUint64Array(isl.flatMap(({ islands: list }) => list)),
            affineF64,
          )
        }
        if (ids.length > 0 || sketches.length > 0 || islands.length > 0) defMove(ids, sketches, islands)
      } else {
        for (const group of islands) {
          wasmScene.transform_sketch_islands(group.sketch, new BigUint64Array(group.islands), affineF64)
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
      // (c) Draw the mixed shapes' captured edges where they now belong.
      for (const [g, cap] of redraws) redrawCaptured(wasmScene, g, cap, [affineF64], label)
      if (activeInstance != null) {
        if (hasPost) {
          wasmScene.transform_def_selection(
            activeInstance,
            new BigUint64Array(0),
            new BigUint64Array(post.sketches),
            new BigUint64Array(post.islands.flatMap(({ sketch, islands: list }) => list.map(() => sketch))),
            new BigUint64Array(post.islands.flatMap(({ islands: list }) => list)),
            affineF64,
          )
        }
      } else {
        for (const group of post.islands) {
          wasmScene.transform_sketch_islands(group.sketch, new BigUint64Array(group.islands), affineF64)
        }
        if (post.sketches.length > 0) {
          wasmScene.transform_selection(new Uint8Array(0), new BigUint64Array(0), new BigUint64Array(post.sketches), affineF64)
        }
      }
      // Re-resolve the moved shapes BY GEOMETRY in the final state.
      for (const g of [...pureDrafts, ...redraws.keys()]) {
        found.set(g, refsAtAnchors(wasmScene, g.ctx, movedAnchors(g, affineF64)))
      }
      for (const g of detaching) {
        found.set(g, refsAtAnchors(wasmScene, ctxThroughAffine(g.ctx, affineF64), movedAnchors(g, affineF64)))
      }
      for (const { node, segs } of riding) {
        if (!isDraftRef(node) || segs.length === 0) continue
        const anchors = segs.map(([a, b]) => [applyAffineToPoint(affineF64, a), applyAffineToPoint(affineF64, b)] as [V3, V3])
        ridingFound.set(node, refsAtAnchors(wasmScene, ridingCtx(node.ctx, affineF64), anchors))
      }
    },
    steps > 1,
  )
  repointGroups(committed, groups, found)
  if (ridingFound.size > 0) {
    const seen = new Set(
      committed.filter((n) => isDraftRef(n) && !ridingFound.has(n)).map((n) => `${n.ctx}:${n.key}`),
    )
    const out: NodeRef[] = []
    for (const node of committed) {
      const refs = ridingFound.get(node)
      if (refs === undefined) {
        out.push(node)
        continue
      }
      for (const ref of refs) {
        const k = `${ref.ctx}:${ref.key}`
        if (seen.has(k)) continue
        seen.add(k)
        out.push(ref)
      }
    }
    committed.splice(0, committed.length, ...out)
  }
  return committed
}

/** Where a shape riding on a moved owner is listed afterwards: its (world)
 *  plane moved through `affine`, keeping an instance context's instance and
 *  an object-scoped context's object. */
function ridingCtx(ctx: string, affine: Float64Array): string | null {
  const pl = planeOfCtx(ctx)
  if (pl === null) return null
  const moved = ctxThroughAffine(ctx, affine)
  if (moved === null) return null
  let object: unknown
  try {
    object = (JSON.parse(ctx) as { object?: unknown }).object
  } catch {
    object = undefined
  }
  if (typeof object !== 'number') return moved
  const mp = planeOfCtx(moved)
  return mp === null ? null : ctxOfObjectPlane([...mp.point, ...mp.normal], BigInt(object))
}

/** The draft ref (island) of a sketch's island, by its edges' positions —
 *  how a copy made through the sketch machinery is handed back as a drawn
 *  shape. `null` when it cannot be found. */
function draftRefOfSketchIsland(wasmScene: WasmScene, sketch: bigint, island: bigint): NodeRef | null {
  const ctx = ctxOfSketch(wasmScene, sketch)
  if (ctx === null) return null
  const anchors: [V3, V3][] = []
  for (const edge of wasmScene.sketch_island_edges(sketch, island)) {
    const ends = wasmScene.sketch_edge_endpoints(sketch, edge)
    if (ends === undefined) continue
    anchors.push([[ends[0], ends[1], ends[2]], [ends[3], ends[4], ends[5]]])
  }
  return refsAtAnchors(wasmScene, ctx, anchors)[0] ?? null
}

/**
 * Lay copies of a selection's owners (objects, groups, instances) and of its
 * drawn shapes at `affines`. `copyOwners` performs the owners' copy (one
 * kernel call) and returns the created refs.
 *
 * The whole gesture is ONE undo step ("Copy") and atomic: with owners and
 * drawing both present it runs in a history bracket, and any refusal puts the
 * document back exactly. The order is deterministic. Whole drawn shapes and
 * sketches copy first (`draft_copy_edges`); then the owners copy — the drawing
 * on their faces rides with the copies; then a MIXED shape (part of it rides
 * with an owner, the rest is drawn) draws its captured non-riding edges again
 * at every rep, circles kept as circles under a similarity.
 */
export function copyOwnersAndDrawing(
  wasmScene: WasmScene,
  owners: readonly NodeRef[],
  drawn: readonly NodeRef[],
  affines: readonly Float64Array[],
  copyOwners: () => NodeRef[],
): NodeRef[] {
  if (drawn.length === 0) return owners.length > 0 ? copyOwners() : []
  if (owners.length === 0) return duplicateSketchSelectionByAffineArray(wasmScene, drawn, affines)
  return inHistoryBracket(wasmScene, t`Copy`, () => {
    const plan = planCopy(wasmScene, [...owners, ...drawn], affines)
    const whole = copyWholeDrawing(wasmScene, plan, affines)
    const ownerCopies = copyOwners()
    return [...whole, ...copyMixedDrawing(wasmScene, plan, affines), ...ownerCopies]
  })
}

/** What a copy of a selection's drawing does, decided before anything lands. */
interface CopyPlan {
  /** In-plane shapes drawn whole: copied with `draft_copy_edges`. */
  pure: DraftGroup[]
  /** In-plane shapes part of which rides with an owner: drawn again. */
  mixed: Map<DraftGroup, CapturedDrawing>
  /** Free islands of shapes that leave their plane: detach-copied. */
  detachIslands: SketchIslandGroup[]
  wholeSketchSources: NodeRef[]
}

function planCopy(wasmScene: WasmScene, selection: readonly NodeRef[], affines: readonly Float64Array[]): CopyPlan {
  const plan: CopyPlan = {
    pure: [],
    mixed: new Map(),
    detachIslands: [],
    wholeSketchSources: selection.filter((n) => n.kind === 'sketch'),
  }
  for (const g of planDraftTransforms(wasmScene, selection)) {
    if (affines.every((a) => inPlane(g, a))) {
      if (g.mixed) plan.mixed.set(g, captureDrawing(wasmScene, g))
      else plan.pure.push(g)
    } else if (g.edges.every((e) => e.store.kind === 'free')) {
      plan.detachIslands.push(...freeIslandsOf(wasmScene, g))
    } else {
      throw notInPlane()
    }
  }
  return plan
}

/** The shapes of a plane context copied by `draft_copy_edges`, and the whole
 *  sketches / detaching islands. */
function copyWholeDrawing(wasmScene: WasmScene, plan: CopyPlan, affines: readonly Float64Array[]): NodeRef[] {
  const committed: NodeRef[] = []
  for (const g of plan.pure) {
    const seen = new Set<string>()
    const addRefs = (affine: Float64Array) => {
      for (const ref of refsAtAnchors(wasmScene, g.ctx, movedAnchors(g, affine))) {
        const k = `${ref.ctx}:${ref.key}`
        if (seen.has(k)) continue
        seen.add(k)
        committed.push(ref)
      }
    }
    // ALL reps of one context go in ONE draft — `draft_copy_edges` once per
    // rep over the original shapes' keys. A rep landing across the originals
    // re-keys them inside the draft and the next rep's keys go stale; the
    // draft is then cancelled whole and the reps are laid down one draft
    // each, re-finding the originals by geometry between reps.
    const original = g.edges.map((e) => e.key)
    try {
      runDraft(wasmScene, { kind: 'ctx', ctx: g.ctx }, t`Copy`, (d) => {
        for (const affine of affines) wasmScene.draft_copy_edges(d, keysToArray(original), affine)
      })
      for (const affine of affines) addRefs(affine)
      continue
    } catch (err) {
      if (affines.length === 1) throw err
    }
    for (const affine of affines) {
      const pu = readPlaneUnits(wasmScene, g.ctx)
      if (pu === null) continue
      const anchors = g.edges.map((e) => [e.a, e.b] as [V3, V3])
      const keys = unitsAtSegments(pu, anchors).flatMap((u) => u.edges.map((e) => e.key))
      if (keys.length === 0) continue
      runDraft(wasmScene, { kind: 'ctx', ctx: g.ctx }, t`Copy`, (d) => {
        wasmScene.draft_copy_edges(d, keysToArray(keys), affine)
      })
      addRefs(affine)
    }
  }
  if (plan.wholeSketchSources.length > 0 || plan.detachIslands.length > 0) {
    committed.push(...duplicateWholeSketchesByAffineArray(wasmScene, plan.wholeSketchSources, affines, plan.detachIslands))
  }
  return committed
}

/** The mixed shapes' captured non-riding edges, drawn at every rep (the
 *  originals stay). */
function copyMixedDrawing(wasmScene: WasmScene, plan: CopyPlan, affines: readonly Float64Array[]): NodeRef[] {
  const committed: NodeRef[] = []
  for (const [g, cap] of plan.mixed) {
    redrawCaptured(wasmScene, g, cap, affines, t`Copy`)
    const seen = new Set<string>()
    for (const affine of affines) {
      for (const ref of refsAtAnchors(wasmScene, g.ctx, movedAnchors(g, affine))) {
        const k = `${ref.ctx}:${ref.key}`
        if (seen.has(k)) continue
        seen.add(k)
        committed.push(ref)
      }
    }
  }
  return committed
}

/**
 * Duplicate a selection at an offset — Move+Alt's copy path for drawn
 * shapes and whole sketches. A translation is just an affine, so this is
 * `duplicateSketchSelectionByAffineArray` with one translation.
 */
export function duplicateSketchSelection(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  delta: [number, number, number],
): NodeRef[] {
  return duplicateSketchSelectionByAffineArray(wasmScene, selection, [
    affineToFloat64(translationAffine(delta[0], delta[1], delta[2])),
  ])
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
 * (or a whole-sketch selection) copy the whole sketch. The whole copy
 * is ONE undo step and atomic: it runs in a history bracket (see
 * `duplicateSketchSelectionByAffineArray`), so a refused copy puts the
 * document back exactly as it found it, whether the failure came from an
 * in-plane replay or an out-of-plane detach-copy.
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
 * cumulative affines — one per copy — for Rotate+Alt's and Move+Alt's ×N /
 * ÷N array refinement (`[stepAffine, 2·stepAffine, …]`, already composed by
 * the caller). Copies drawn shapes and whole sketches.
 *
 * Drawn shapes (`draft-*` refs) group by plane context, edges and curves
 * lifting to their shape. Routing is decided ONCE for the whole array:
 *
 * - every rep in the plane → ONE draft per context (one undo step for the
 *   whole array), `draft_copy_edges` once per rep over the shapes' edge keys,
 *   the originals left. If a rep landing across the originals re-keys them
 *   mid-draft, the draft is cancelled and each rep gets its own draft,
 *   re-reading the originals' keys by geometry. Copies arrange like a draw,
 *   so a copy half over a face and half over the void is fine. A MIXED shape
 *   (part of it rides with a selected owner) is not copied by key: its
 *   non-riding edges are captured and drawn again at every rep.
 * - any rep leaves the plane: a pure-void shape (every edge free) copies
 *   onto new sketches through the kernel's detach machinery
 *   (`copy_sketch_islands`, per rep); a shape with any face edge refuses
 *   `NotInPlane`, before anything lands.
 *
 * Whole sketches replay as they always did (data-driven in-plane vs
 * out-of-plane routing). The call is ONE undo step ("Copy") and ATOMIC: when
 * it could record more than one entry it runs in a history bracket, and any
 * throw restores the document exactly.
 *
 * Returns the copies as selection refs (drawn shapes as `draft-island` refs
 * found by geometry) so the caller can reselect them.
 */
export function duplicateSketchSelectionByAffineArray(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  affines: readonly Float64Array[],
): NodeRef[] {
  const plan = planCopy(wasmScene, selection, affines)
  const sketchPart = plan.wholeSketchSources.length > 0 || plan.detachIslands.length > 0
  const steps = plan.pure.length + plan.mixed.size + (affines.length > 1 && plan.pure.length > 0 ? 1 : 0)
  return inHistoryBracket(
    wasmScene,
    t`Copy`,
    () => [...copyWholeDrawing(wasmScene, plan, affines), ...copyMixedDrawing(wasmScene, plan, affines)],
    sketchPart || steps > 1,
  )
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
 * object array's `duplicate_selection_array(count=N)` shape . Out-of-plane
 * reps have no such kernel primitive to batch through — each
 * `copy_sketch_islands` call is its own atomic step; the caller's history
 * bracket folds however many of those (and per-sketch replays) into the one
 * undo step of the gesture.
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
 * (or a whole-sketch selection) copy the whole sketch. The whole copy
 * is ONE undo step and atomic: it runs in a history bracket (see
 * `duplicateSketchSelectionByAffineArray`), so a refused copy puts the
 * document back exactly as it found it, whether the failure came from an
 * in-plane replay or an out-of-plane detach-copy.
 *
 * Returns the copies as `sketch-island` NodeRefs so the caller can reselect
 * them, matching `duplicateSketchSelection`'s return shape.
 */
function duplicateWholeSketchesByAffineArray(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  affines: readonly Float64Array[],
  freeIslands: readonly SketchIslandGroup[] = [],
): NodeRef[] {
  const { sketches, islands } = planSketchTransforms(wasmScene, selection, freeIslands)

  // Islands to copy, per sketch: whole-sketch selections take every island.
  const islandsBySketch = new Map<bigint, bigint[]>()
  for (const sketch of sketches) {
    islandsBySketch.set(sketch, [...wasmScene.sketch_island_ids(sketch)])
  }
  for (const { sketch, islands: ids } of islands) {
    const list = islandsBySketch.get(sketch) ?? []
    list.push(...ids)
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
  for (const [sketch, repPlans] of inPlanePlan) {
    // Re-fetch: a pure query, and the plane used to classify this sketch
    // above (needed again here to project points exactly onto it).
    const plane = wasmScene.sketch_plane(sketch)
    if (plane === undefined) continue // stale — vanishingly unlikely mid-call, never crash on it

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
    }

    // Map the replayed edges to their (possibly merged) islands.
    const seen = new Set<string>()
    for (const edge of newEdges) {
      const island = wasmScene.sketch_edge_island(sketch, edge)
      if (island === undefined) continue // split away later in the replay
      const key = island.toString()
      if (seen.has(key)) continue
      seen.add(key)
      const ref = draftRefOfSketchIsland(wasmScene, sketch, island)
      if (ref !== null) committed.push(ref)
    }
  }

  // Out-of-plane (or orientation-flipping, or margin-failing) copies: each
  // source sketch's islands land TOGETHER on one new sketch via the
  // kernel's detach/rebuild machinery, source left untouched — same
  // reasoning as `duplicateSketchSelection`'s out-of-plane branch (a
  // region's hole boundary is its own island). One `copy_sketch_islands`
  // call PER REP (no kernel primitive batches these, unlike the in-plane
  // bracket above), so N reps are N kernel steps here, folded into the gesture's one
  // undo step by the caller's history bracket.
  for (const [sketch, islandIds] of copyBySketch) {
    if (islandIds.length === 0) continue
    for (const affineF64 of affines) {
      const copySketch = wasmScene.copy_sketch_islands(
        sketch,
        new BigUint64Array(islandIds),
        affineF64,
      )
      for (const copyIsland of wasmScene.sketch_island_ids(copySketch)) {
        const ref = draftRefOfSketchIsland(wasmScene, copySketch, copyIsland)
        if (ref !== null) committed.push(ref)
      }
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
    // The wrapper is what the tool drives, so it is the ghost's root for
    // the sibling preview mirror (markSessionGhost), not the posed leaf.
    const wrapper = new THREE.Group()
    wrapper.add(posed)
    return markSessionGhost(wrapper)
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
  if (isDraftKind(node.kind)) {
    // A drawn shape: the ghost is its own line work (already WORLD
    // coordinates, instanced or not). Edges and curves preview the shape
    // (island) they ride with, matching exactly what the commit will move.
    const r = resolveDraft(wasmScene, node)
    if (r === null || r.unit === null) return null // stale — nothing to preview
    const out = new Float32Array(r.unit.edges.length * 6)
    r.unit.edges.forEach((e, i) => out.set([e.a[0], e.a[1], e.a[2], e.b[0], e.b[1], e.b[2]], i * 6))
    return buildSketchPreviewClone(out)
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
  // The shared group is the one object the tool drives — the ghost root
  // the sibling preview mirror clones and follows (markSessionGhost).
  return group.children.length > 0 ? markSessionGhost(group) : null
}
