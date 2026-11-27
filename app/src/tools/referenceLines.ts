/**
 * Reference lines on the drawing plane (wireframe-faces-and-solids design §3).
 *
 * While a shape is being drawn, points that are not on the drawing plane P
 * still say where things line up: the corner of the shape the gesture started
 * on, the gesture's own earlier points, a point the cursor rested on. Each
 * such **reference point** R has a **foot** R′ (its perpendicular projection
 * onto P, R itself when R is on P) and offers two lines through the foot, along
 * the two directions a shape's sides take on P. The cursor snaps to those
 * lines and to where they cross. The kind is `'from-point'`: Line's "draw
 * three sides of a square and the fourth snaps shut" is the same inference
 * with the gesture's own points as the only source, and this module is that
 * inference generalized and shared by the five draw tools.
 *
 * Reference points come from three sources, merged and de-duplicated by foot:
 *   1. the gesture's own points (a Line chain's earlier vertices, an Arc's
 *      endpoints) — passed in by the tool on every resolve;
 *   2. the corners of the shape the gesture started on — one read-only wasm
 *      call (`snap_owner_points`) at the first click, feature-checked so the
 *      app works before the binding exists. A plain sketch corner's snap
 *      names no sketch, so the corner under the click ray is picked
 *      (`pick_sketch_vertex`) and accepted only where it is the snap point
 *      (ruling R15); at a corner several sketches share, the call answers
 *      with every one of their islands (a wall standing on a ground
 *      Rectangle lends its top corners to the next wall started there);
 *   3. the dwell ring: a named point the cursor stayed snapped to for
 *      `REFERENCE_DWELL_MS`, the last `REFERENCE_RING_SIZE` of them. It clears
 *      on a tool change and on Esc from idle (the tools call `clearRing`).
 *
 * Rank: a reference-line snap replaces the engine's answer only when that
 * answer is null or weak (`FROM_POINT_WEAK_SNAP_KINDS`). A real point or an
 * edge under the cursor still wins. Under a hard axis lock the candidate is
 * where the lock ray meets the reference line (`fromPointCandidate`).
 *
 * A plain projected snap (a named point dropped onto the plane, design §2.3)
 * keeps its kind and its "projected" disclosure; `projectedSnap` adds the
 * tie line's source so the cue shows where it came from.
 */

import { WORLD_DRAWING_AXES, getDrawingAxes, type DrawingAxes } from './drawingAxes'
import type { Scene as WasmScene } from '../wasm/loader'
import { isNamedPointSnap } from './drawPlane'
import { dotV3, fromPointCandidate, segmentLength, type V3 } from './lineInput'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'

/** Screen-pixel radius within which a reference line takes over the cursor. */
export const FROM_POINT_SNAP_PX = 8

/** World-space fallback tolerance before the first `updateDiskScale` tick has
 *  landed (no live camera feed yet — every unit test that never wires a
 *  camera in). */
export const FROM_POINT_FALLBACK_TOLERANCE_M = 0.02

/**
 * Kernel/inference snap kinds weak enough that a reference line is allowed to
 * override them. A precise kernel snap (endpoint, midpoint, center,
 * intersection, quadrant, tangent, on-edge, on-guide, ...) always wins over
 * this app-side inference — the override only fires when the resolved snap
 * carries no such commitment: no snap at all, a bare constraint-plane/ground
 * fallback, an on-face hover, or the kernel's own SOFT on-axis inference.
 */
export const FROM_POINT_WEAK_SNAP_KINDS: ReadonlySet<string> = new Set(['plane', 'ground', 'on-face', 'on-axis'])

/** How long the cursor must stay snapped to a named point before it becomes a
 *  reference point (design §3.2, source 3). */
export const REFERENCE_DWELL_MS = 300

/** How many rested-on points the ring keeps. */
export const REFERENCE_RING_SIZE = 4

/** How many of the start shape's corners are kept, nearest the anchor. */
export const MAX_OWNER_POINTS = 64

/** Feet closer than this (metres) are one foot — the kernel's own point-merge
 *  scale, far above float noise and far below any drawn dimension. */
const FOOT_MERGE_EPS = 1e-9

/** A point this close to the plane (metres) is on it: the kernel's
 *  `tol::PLANE_DIST`, as `GROUND_PLANE_EPS` mirrors it everywhere else. */
const ON_PLANE_EPS = 1e-9

/** Two unit directions with |sin| below this are parallel. */
const PARALLEL_EPS = 1e-9

/** The plane a gesture draws on: all this module needs of a `DrawPlane`. */
export interface ReferencePlane {
  origin: V3
  normal: V3
  u: V3
  v: V3
}

/** One reference line: the infinite line through `foot` along `dir`. */
export interface ReferenceLine {
  /** The foot R′ on the plane. */
  foot: V3
  /** R when R is off the plane (the tie line's far end), else undefined. */
  source?: V3
  /** Unit direction in the plane. */
  dir: V3
}

/** What `resolveReferenceSnap` hands back: the point and the cue snap. */
export interface ReferenceResolution {
  point: V3
  snap: Snap
}

function norm(d: V3): V3 {
  const len = Math.hypot(d[0], d[1], d[2])
  return len > 0 ? [d[0] / len, d[1] / len, d[2] / len] : d
}

/** The scene's drawing frame, or the world's when the scene has none to give
 *  (a bare test double). */
export function drawingFrame(scene: WasmScene): DrawingAxes {
  return typeof scene.axes === 'function' ? getDrawingAxes(scene) : WORLD_DRAWING_AXES
}

/** The two directions a shape's sides take on `plane`: the frame axes that lie
 *  in it (an axis plane), else the plane's own basis pair (`facePlaneBasis`). */
export function planeDirections(plane: ReferencePlane, frame: DrawingAxes): [V3, V3] {
  const n = norm(plane.normal)
  const inPlane = [frame.x, frame.y, frame.z].filter((a) => Math.abs(dotV3(norm(a), n)) <= PARALLEL_EPS)
  if (inPlane.length >= 2) return [inPlane[0], inPlane[1]]
  return [plane.u, plane.v]
}

/** Signed distance of `p` off `plane`. */
function offPlane(p: V3, plane: ReferencePlane): number {
  const n = norm(plane.normal)
  return (p[0] - plane.origin[0]) * n[0] + (p[1] - plane.origin[1]) * n[1] + (p[2] - plane.origin[2]) * n[2]
}

/** Foot of `p` on `plane` and whether `p` was off it. On the plane the foot is
 *  `p` itself, bit for bit. */
export function footOnPlane(p: V3, plane: ReferencePlane): { foot: V3; off: boolean } {
  const d = offPlane(p, plane)
  if (Math.abs(d) <= ON_PLANE_EPS) return { foot: p, off: false }
  const n = norm(plane.normal)
  return { foot: [p[0] - n[0] * d, p[1] - n[1] * d, p[2] - n[2] * d], off: true }
}

/**
 * The reference lines for `points` on `plane`: two per distinct foot, along
 * `dirs`. Feet are de-duplicated (a point on the plane beats an off-plane
 * point with the same foot), and a foot at `skip` (the gesture's anchor, whose
 * axis lines the engine already offers) offers nothing.
 */
export function referenceLines(
  points: readonly V3[],
  plane: ReferencePlane,
  dirs: readonly V3[],
  skip: V3 | null,
): ReferenceLine[] {
  const feet: { foot: V3; source?: V3 }[] = []
  for (const p of points) {
    const { foot, off } = footOnPlane(p, plane)
    if (skip !== null && segmentLength(foot, skip) <= FOOT_MERGE_EPS) continue
    const at = feet.findIndex((f) => segmentLength(f.foot, foot) <= FOOT_MERGE_EPS)
    if (at >= 0) {
      if (!off && feet[at].source !== undefined) feet[at] = { foot }
      continue
    }
    feet.push(off ? { foot, source: p } : { foot })
  }
  const lines: ReferenceLine[] = []
  for (const f of feet) {
    for (const d of dirs) lines.push({ foot: f.foot, ...(f.source !== undefined ? { source: f.source } : {}), dir: norm(d) })
  }
  return lines
}

/** Perpendicular foot of `cursor` on `line` (built from the line's own foot so
 *  a coordinate the line holds constant keeps the foot's exact bits). */
function nearestOn(line: { foot: V3; dir: V3 }, cursor: V3): V3 {
  const w: V3 = [cursor[0] - line.foot[0], cursor[1] - line.foot[1], cursor[2] - line.foot[2]]
  const t = dotV3(w, line.dir)
  return [line.foot[0] + t * line.dir[0], line.foot[1] + t * line.dir[1], line.foot[2] + t * line.dir[2]]
}

/** Where two in-plane lines cross, or null when they are parallel. */
function crossing(a: { foot: V3; dir: V3 }, b: { foot: V3; dir: V3 }, plane: ReferencePlane): V3 | null {
  const n = norm(plane.normal)
  // 2-D cross product of the directions, signed by the plane normal.
  const c = cross(a.dir, b.dir)
  const denom = dotV3(c, n)
  if (Math.abs(denom) < PARALLEL_EPS) return null
  const w: V3 = [b.foot[0] - a.foot[0], b.foot[1] - a.foot[1], b.foot[2] - a.foot[2]]
  const t = dotV3(cross(w, b.dir), n) / denom
  return [a.foot[0] + t * a.dir[0], a.foot[1] + t * a.dir[1], a.foot[2] + t * a.dir[2]]
}

function cross(a: V3, b: V3): V3 {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

export interface ResolveInput {
  /** The engine's answer (before any plane projection). */
  snap: Snap | null
  /** The cursor's point as the tool computed it (on the plane for the frozen
   *  tools; Line may hand an off-plane soft point). */
  cursor: V3
  plane: ReferencePlane
  /** The gesture's anchor (segment start); null before the first click. */
  anchor: V3 | null
  /** Unit direction of a hard axis lock from the anchor (Line only). */
  lockDir: V3 | null
  /** The drawing frame, read only when there are reference points to use. */
  frame: () => DrawingAxes
  /** Reference points, all three sources merged. */
  points: readonly V3[]
  /** World-space acquire radius at a point. */
  tolerance: (at: V3) => number
}

/** One accepted candidate, before it becomes a `Snap`. */
interface Hit {
  point: V3
  /** The reference line it lies on. */
  line: ReferenceLine
  /** Second line, for a crossing. */
  also?: { foot: V3; source?: V3; dir: V3 }
  distance: number
}

/**
 * The reference-line snap for this cursor, or null when the engine's answer
 * outranks it or nothing lies within tolerance.
 *
 * Unlocked: the cursor snaps to the nearest reference line within tolerance;
 * a crossing (of two reference lines, or of a reference line with an axis line
 * from the anchor) wins over a single line when the cursor is within
 * tolerance of both lines, and snaps to the exact crossing. Locked: the
 * candidate is where the lock ray meets each line, nearest to the cursor.
 */
export function resolveReferenceSnap(input: ResolveInput): ReferenceResolution | null {
  const { snap, cursor, plane, anchor, lockDir, frame, points, tolerance } = input
  if (snap !== null && !FROM_POINT_WEAK_SNAP_KINDS.has(snap.kind)) return null
  if (points.length === 0) return null

  const dirs = planeDirections(plane, frame())
  const lines = referenceLines(points, plane, dirs, anchor)
  if (lines.length === 0) return null

  let best: Hit | null = null

  if (lockDir !== null) {
    if (anchor === null) return null
    for (const line of lines) {
      const candidate = fromPointCandidate(line.foot, line.dir, anchor, cursor, lockDir, plane.origin, plane.u, plane.v)
      if (candidate === null) continue
      const distance = segmentLength(candidate, cursor)
      if (distance > tolerance(candidate)) continue
      if (best === null || distance < best.distance) best = { point: candidate, line, distance }
    }
  } else {
    const near = (line: { foot: V3; dir: V3 }): number => segmentLength(nearestOn(line, cursor), cursor)
    let bestLine: Hit | null = null
    for (const line of lines) {
      const point = nearestOn(line, cursor)
      const distance = segmentLength(point, cursor)
      if (distance > tolerance(point)) continue
      if (bestLine === null || distance < bestLine.distance) bestLine = { point, line, distance }
    }
    // Crossings: reference × reference, and reference × an axis line from the
    // anchor. Each needs the cursor within tolerance of BOTH lines.
    const axisLines = anchor === null ? [] : dirs.map((d) => ({ foot: anchor, dir: norm(d) }))
    const crosses: Hit[] = []
    const consider = (line: ReferenceLine, other: { foot: V3; source?: V3; dir: V3 }): void => {
      const x = crossing(line, other, plane)
      if (x === null) return
      const tol = tolerance(x)
      if (near(line) > tol || near(other) > tol) return
      crosses.push({ point: x, line, also: other, distance: segmentLength(x, cursor) })
    }
    for (let i = 0; i < lines.length; i++) {
      for (let j = i + 1; j < lines.length; j++) consider(lines[i], lines[j])
      for (const axis of axisLines) consider(lines[i], axis)
    }
    const bestCross = crosses.reduce<Hit | null>((m, h) => (m === null || h.distance < m.distance ? h : m), null)
    best = bestCross ?? bestLine
  }

  if (best === null) return null
  const { point, line, also } = best
  const out: Snap = {
    x: point[0], y: point[1], z: point[2],
    kind: 'from-point',
    direction: line.dir,
    from: line.foot,
    ...(line.source !== undefined ? { fromSource: line.source } : {}),
    ...(also !== undefined
      ? { fromAlso: { from: also.foot, direction: also.dir, ...(also.source !== undefined ? { fromSource: also.source } : {}) } }
      : {}),
  }
  return { point, snap: out }
}

/**
 * The cue snap for a plain projected snap (design §2.3): the engine's snap
 * dropped onto the plane at `foot`. It keeps its kind (the chip still says
 * "projected") and gains `fromSource`, the point it came from, so the cue
 * draws the tie line. Returns `snap` itself when nothing moved.
 */
export function projectedSnap(snap: Snap | null, foot: V3): Snap | null {
  if (snap === null) return null
  const raw: V3 = [snap.x, snap.y, snap.z]
  if (segmentLength(raw, foot) <= FOOT_MERGE_EPS) return snap
  return { ...snap, x: foot[0], y: foot[1], z: foot[2], fromSource: raw }
}

/** The wasm read `snap_owner_points` as the app sees it (flat xyz, world). */
type OwnerPointsFn = (
  sketch: bigint | undefined,
  object: bigint | undefined,
  element: bigint | undefined,
  elementKind: string | undefined,
  instance: bigint | undefined,
) => Float64Array

/** How far a picked sketch vertex may lie from an ownerless snap point and
 *  still be that point (ruling R15): the same point, up to float noise. */
export const OWNER_PICK_EPS_M = 1e-9

/** The wasm read `pick_sketch_vertex`'s result as the app sees it. */
interface SketchVertexPick {
  sketch(): bigint
  vertex(): bigint
  x(): number
  y(): number
  z(): number
  free?(): void
}

/** The sketch vertex an ownerless named-point snap sits on (ruling R15): the
 *  vertex `pick_sketch_vertex` finds on the click `ray`, accepted only when
 *  it is the snap point. `null` when there is none, it is elsewhere (a guide
 *  point, a solid's corner), or the binding is absent or throws. */
function pickedCorner(scene: unknown, snap: Snap, ray: Ray): { sketch: bigint; vertex: bigint } | null {
  const pick = (scene as { pick_sketch_vertex?: unknown }).pick_sketch_vertex
  if (typeof pick !== 'function') return null
  let hit: SketchVertexPick | undefined
  try {
    hit = (pick as (...a: number[]) => SketchVertexPick | undefined).call(
      scene,
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
  } catch {
    return null
  }
  if (hit === undefined || hit === null) return null
  try {
    const at: V3 = [hit.x(), hit.y(), hit.z()]
    if (segmentLength(at, [snap.x, snap.y, snap.z]) > OWNER_PICK_EPS_M) return null
    return { sketch: hit.sketch(), vertex: hit.vertex() }
  } catch {
    return null
  } finally {
    hit.free?.()
  }
}

/** The shape's corners that the first click's snap sits on, nearest `anchor`
 *  first, at most `MAX_OWNER_POINTS`. A named point with no owner (a plain
 *  sketch corner) is resolved through the click `ray` (ruling R15). Empty
 *  when the snap has no owner or a binding is not in this build of the wasm
 *  package. */
export function ownerPoints(scene: unknown, snap: Snap | null, anchor: V3, ray?: Ray): V3[] {
  if (snap === null) return []
  let args: [bigint | undefined, bigint | undefined, bigint | undefined, string | undefined]
  if (snap.sketch !== undefined || snap.object !== undefined) {
    args = [snap.sketch, snap.object, snap.element ?? snap.sketchRegion ?? snap.sketchCurve, snap.elementKind]
  } else {
    if (ray === undefined || !isNamedPointSnap(snap)) return []
    const corner = pickedCorner(scene, snap, ray)
    if (corner === null) return []
    args = [corner.sketch, undefined, corner.vertex, 'sketch-vertex']
  }
  const fn = (scene as { snap_owner_points?: unknown }).snap_owner_points
  if (typeof fn !== 'function') return []
  let flat: Float64Array
  try {
    flat = (fn as OwnerPointsFn).call(scene, ...args, snap.instance)
  } catch {
    return []
  }
  const pts: V3[] = []
  for (let i = 0; i + 2 < flat.length; i += 3) pts.push([flat[i], flat[i + 1], flat[i + 2]])
  pts.sort((a, b) => segmentLength(a, anchor) - segmentLength(b, anchor))
  return pts.slice(0, MAX_OWNER_POINTS)
}

/**
 * Per-tool reference-point state and the screen-scale feed: the dwell ring
 * (source 3), the start shape's corners (source 2), and the world-per-pixel
 * conversion for the acquire radius. The gesture's own points (source 1) are
 * passed to `resolve` by the tool, which is the only one that knows them.
 */
export class ReferenceLines {
  private ring: V3[] = []
  private pending: { point: V3; since: number; credited: boolean } | null = null
  private owner: V3[] = []
  private worldPerPixelFn: ((dist: number) => number) | null = null
  private cameraPos: V3 | null = null

  /** Live pixel-scale feed (`Tool.updateDiskScale`). */
  updateScale(camera: { position: { x: number; y: number; z: number } }, worldPerPixel: (dist: number) => number): void {
    this.worldPerPixelFn = worldPerPixel
    this.cameraPos = [camera.position.x, camera.position.y, camera.position.z]
  }

  /** World-space acquire radius at `at` (`FROM_POINT_SNAP_PX` on screen). */
  tolerance = (at: V3): number => {
    if (this.worldPerPixelFn === null || this.cameraPos === null) return FROM_POINT_FALLBACK_TOLERANCE_M
    return this.worldPerPixelFn(segmentLength(at, this.cameraPos)) * FROM_POINT_SNAP_PX
  }

  /** Dwell tracking, called on every pointer move with the engine's snap.
   *  A named point the cursor stays snapped to for `REFERENCE_DWELL_MS`
   *  joins the ring; so does one the cursor left after resting on it for that
   *  long between two events (a resting cursor sends none). */
  observeHover(snap: Snap | null, now: number = performance.now()): void {
    const point: V3 | null = isNamedPointSnap(snap) ? [snap.x, snap.y, snap.z] : null
    const p = this.pending
    if (p !== null) {
      const same = point !== null && segmentLength(point, p.point) <= FOOT_MERGE_EPS
      if (now - p.since >= REFERENCE_DWELL_MS && !p.credited) {
        this.remember(p.point)
        p.credited = true
      }
      if (same) return
    }
    this.pending = point === null ? null : { point, since: now, credited: false }
  }

  private remember(point: V3): void {
    const at = this.ring.findIndex((q) => segmentLength(q, point) <= FOOT_MERGE_EPS)
    if (at >= 0) this.ring.splice(at, 1)
    this.ring.push(point)
    if (this.ring.length > REFERENCE_RING_SIZE) this.ring.shift()
  }

  /** The rested-on points, oldest first. */
  get dwellPoints(): readonly V3[] {
    return this.ring
  }

  /** Forget the ring and any dwell in progress (tool change, Esc from idle). */
  clearRing(): void {
    this.ring = []
    this.pending = null
  }

  /** A gesture's first click: read the start shape's corners (`ray`, the
   *  click ray, resolves a plain sketch corner's owner). */
  beginGesture(scene: unknown, snap: Snap | null, anchor: V3, ray?: Ray): void {
    this.owner = ownerPoints(scene, snap, anchor, ray)
  }

  /** The gesture ended. */
  endGesture(): void {
    this.owner = []
  }

  /** All three sources for one resolve. */
  points(gesture: readonly V3[]): V3[] {
    return [...gesture, ...this.owner, ...this.ring]
  }

  /**
   * The reference-line snap for this cursor (`resolveReferenceSnap`) with the
   * tool's gesture points added to the owner corners and the dwell ring.
   */
  resolve(input: Omit<ResolveInput, 'points' | 'tolerance'> & { gesture: readonly V3[] }): ReferenceResolution | null {
    const { gesture, ...rest } = input
    return resolveReferenceSnap({ ...rest, points: this.points(gesture), tolerance: this.tolerance })
  }
}

/** What a draw tool's plane cursor comes to once reference lines are consulted. */
export interface DrawCursor {
  point: V3
  snap: Snap | null
  /** True when a reference line (not the engine's snap) decided the point:
   *  whatever "projected" verdict the engine's snap had no longer applies. */
  reference: boolean
}

/**
 * The shared tail of every draw tool's plane cursor: `cursor` is what the
 * tool's own `_planeCursor` made of `snap`; `projected` is its verdict that
 * it dropped the snap onto the plane. A reference line that wins replaces the
 * point and carries its own cue; otherwise the point stands, and a projected
 * snap gains the tie line's source (`projectedSnap`).
 */
export function resolveDrawCursor(
  refs: ReferenceLines,
  input: Omit<ResolveInput, 'points' | 'tolerance'> & { gesture: readonly V3[]; projected: boolean },
): DrawCursor {
  const { projected, ...rest } = input
  const hit = refs.resolve(rest)
  if (hit !== null) return { point: hit.point, snap: hit.snap, reference: true }
  return { point: input.cursor, snap: projected ? projectedSnap(input.snap, input.cursor) : input.snap, reference: false }
}

/**
 * The reference-line snap for an IDLE hover or a first click (design §3.2,
 * source 3: the dwell ring is the only source that works before the first
 * click). `cursor` is where the click would land on `plane` (the plane that
 * click would freeze); the ring's points are projected onto that plane and
 * the cursor snaps to their lines. Null when nothing applies, so the caller
 * keeps the engine's snap.
 */
export function resolveIdleReference(
  refs: ReferenceLines,
  scene: WasmScene,
  input: { snap: Snap | null; cursor: V3; plane: ReferencePlane; projected: boolean },
): DrawCursor | null {
  if (refs.dwellPoints.length === 0) return null
  const out = resolveDrawCursor(refs, {
    snap: input.snap, cursor: input.cursor, plane: input.plane, anchor: null, lockDir: null,
    frame: () => drawingFrame(scene), gesture: [], projected: input.projected,
  })
  return out.reference ? out : null
}
