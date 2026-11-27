/**
 * followMeStart — whether the profile a Follow Me cursor is hovering would be
 * accepted as a legal START for the picked path, and — since auto-orientation
 * (design §2c) — whether it will simply be stood upright automatically
 * instead.
 *
 * WHY THIS EXISTS. Follow Me's start rule is invisible: the kernel accepts a
 * profile only when the profile's PLANE meets the path in a very particular
 * way, or else it FOLDS the profile upright and retries once (design §2c) —
 * and only a decisively wrong placement (a corner fold-back, or a closed
 * path the plane never touches) still comes back as a post-click
 * `[PathTooTight]` / `[PathDetachedFromProfile]` toast. This module is the
 * app-side mirror of that rule, so the tool can warn — or inform — while the
 * user is still hovering a profile, before any click commits to it.
 *
 * (An earlier version of this module also mapped out every legal start on the
 * path itself — quadrant rings on a circle, end rings on an open path, and,
 * once corner seams landed, almost every polyline/face-loop vertex — and drew
 * them on the path before a profile was even chosen. Using the tool in
 * practice showed that cue answers the wrong question: by the time Follow Me
 * is invoked the profile is already placed, and what the user wants to know
 * is whether THIS profile will work, not where one could go. That path-marker
 * layer was removed; only the profile-hover verdict below remains.)
 *
 * THE RULE, as `Object::from_follow_me` (crates/kernel/src/ops.rs) actually
 * implements it — every branch below cites the code it mirrors:
 *
 *   Closed path, segment attributed to a drawn circle. Perpendicularity is
 *   measured against the ANALYTIC tangent, not the facet chord, and the
 *   profile plane is perpendicular to the circle somewhere iff it is RADIAL:
 *   its normal lies in the circle's plane (|n·axis| ≤ NORMAL_DIRECTION) and it
 *   passes through the center (|plane·center| ≤ PLANE_DIST). So for a circle
 *   path the legal placements are the whole one-parameter family of planes
 *   containing the circle's axis.
 *
 *   Closed path, plain segment. The plane's normal must be parallel to the
 *   segment (|n·dir| ≥ 1 − NORMAL_DIRECTION) AND the plane must either cross
 *   the segment's STRICT INTERIOR, or pass through one of its two endpoints —
 *   a CORNER SEAM (design §2b). A corner seam is legal when every point of
 *   the hovered profile sits BEYOND the corner along the OTHER (non-
 *   perpendicular) flank's own direction — `(point − corner) · d ≥
 *   −POINT_MERGE`, `d` being that flank's unit direction pointing INTO the
 *   corner — or when every point sits BEHIND it, inside the loop (a rebate
 *   drawn at a corner of the face it runs around): the kernel then sweeps it
 *   like the same profile slid along the edge to mid-run, refusing
 *   `ProfileTooWideForCorner` only when it is too wide for the loop (mirrored
 *   by transporting the ring around the loop exactly as the sweep does). A
 *   profile on BOTH sides straddles the corner, folds into its own swept
 *   material and refuses `PathTooTight`, never nudged. A vertex where the
 *   path runs straight on (an imprint ending on a face's rim) is no corner.
 *   This is only decidable with the actual profile geometry in hand, which is
 *   why `evaluateStart` takes an optional `profileRing` — without one, a
 *   corner placement can only ever read `'unknown'`.
 *
 *   Closed path, facet joint of ONE drawn curve. A vertex shared by two
 *   segments of the SAME curve chain IS a legal start (the joint's miter plane
 *   is the profile plane) — unconditionally, no fold test, since the joint's
 *   own tangent already IS the miter plane.
 *
 *   Open path. Only the two ENDS can start a sweep: the profile plane must be
 *   perpendicular to the end segment (against the analytic tangent when the
 *   end segment is curve-attributed). The end vertex need NOT sit on the
 *   profile plane — a perpendicular-but-detached end is CARRIED rigidly to
 *   the profile (design §2a), so this is never a refusal on its own; the
 *   `'ok'` verdict carries a `carried` flag so the tool's copy can say the
 *   sweep starts at the profile and follows the path's shape, rather than
 *   implying the path itself sits there.
 *
 * WHAT IT REFUSES TO GUESS. A cue that says "legal here" where the kernel then
 * refuses is worse than no cue, so the verdict is deliberately FOUR-valued:
 *
 *   - `'ok'` is claimed ONLY for f64-exact paths (sketch edges, whose
 *     coordinates cross the WASM boundary as f64 and so reproduce the kernel's
 *     arithmetic) and only under the kernel's own tolerances. A face-loop path
 *     arrives as f32 (`face_boundary`), far coarser than the 1e-9 tolerances,
 *     so `'ok'` is never claimed for one.
 *   - `'orient'` is what a placement that is square to NOTHING on the path
 *     used to earn as a `'refused'` (`ProfileNotPerpendicular`) verdict —
 *     before auto-orientation (design §2c). The kernel now folds a non-
 *     perpendicular profile upright and retries before ever refusing, and
 *     that fold succeeds for essentially any placement near the path (see
 *     `crates/kernel/src/ops.rs`'s `orient_profile_to_path` and its specs),
 *     so a red "this will be refused" cue would now be a LIE for the common
 *     case it used to correctly warn about. `'orient'` is informational, not
 *     a warning: "not square yet, but Follow Me will stand it up for you."
 *   - `'refused'` is claimed only when the placement is DECISIVELY wrong in a
 *     way auto-orientation cannot fix — a corner fold-back (`PathTooTight`,
 *     still a real refusal after the fold) or a closed path the profile's
 *     plane is exactly perpendicular to but does not touch anywhere
 *     (`PathDetachedFromProfile` — item 5 of the SketchUp-parity gap
 *     analysis, explicitly out of scope: "a closed path missed entirely by
 *     the plane"). Both are outside a slop band far wider than either the
 *     kernel tolerance or f32 noise; a placement that is merely a hair off
 *     reports `'unknown'`.
 *   - `'unknown'` is the honest answer in the band between, and for anything
 *     this module cannot chain (a branching or disconnected path).
 *
 * And even `'ok'` (or `'orient'`) means only "the START is valid". The sweep
 * can still refuse for reasons that depend on the whole transport —
 * `PathReverses`, the closed seam's exact re-landing check — that are not
 * decidable from the start placement alone. The tool's copy says "ready to
 * sweep", never "this will work". (A corner seam's own fold refusal,
 * `PathTooTight`, IS decidable from the start placement — see the
 * corner-seam paragraph above — and is the one exception this module
 * predicts.)
 */

import { t } from '@lingui/core/macro'

/** A world point / direction. */
export type Vec3 = readonly [number, number, number]

/** The analytic circle a path segment was drawn from (kernel `CurveGeom`). */
export interface CurveGeom {
  center: Vec3
  radius: number
}

/** One path segment, with the analytic curve it belongs to (if any). */
export interface PathSegment {
  a: Vec3
  b: Vec3
  /** The analytic circle this segment was drawn from, or null when the
   *  segment carries no `CurveGeom` (a plain line, or a curve chain drawn
   *  before geometry capture — the kernel's every curve branch is gated on
   *  the `CurveGeom` being present, so such a chain is plain segments). */
  curve: CurveGeom | null
}

/**
 * The kernel's `same_curve` test: two segments belong to the same drawn circle
 * when their `CurveGeom`s agree, NOT when they carry the same curve id. Two
 * separately-drawn arcs of one circle meeting end to end are a smooth joint
 * the sweep can seam on, and comparing chain identity instead would call that
 * joint a corner and refuse a placement the kernel accepts.
 */
function sameCurve(a: CurveGeom | null, b: CurveGeom | null): boolean {
  if (a === null || b === null) return false
  // Centers compare by EUCLIDEAN distance, because that is what the kernel's
  // `Point3::approx_eq` does (math.rs — `(self - other).length_squared() <=
  // tol * tol`). A per-axis comparison would be a CUBE of half-width
  // POINT_MERGE where the kernel uses a BALL of that radius, and the corners
  // of that cube are outside the ball: an offset of (6e-10, 6e-10, 6e-10)
  // passes per-axis but is 1.04e-9 away, so the app would call a joint
  // "same curve", report the placement `ok`, and the kernel would then refuse
  // it `PathDetachedFromProfile`. Saying "legal here" where the sweep refuses
  // is the one failure this module exists to avoid.
  const dx = a.center[0] - b.center[0]
  const dy = a.center[1] - b.center[1]
  const dz = a.center[2] - b.center[2]
  return (
    dx * dx + dy * dy + dz * dz <= POINT_MERGE * POINT_MERGE &&
    Math.abs(a.radius - b.radius) <= POINT_MERGE
  )
}

/**
 * A chained path, as produced by [`chainPath`]: the segments are in walk order
 * AND consistently oriented, so `segments[k].b === segments[k+1].a` and every
 * joint vertex is the START of exactly one segment. That is the same shape the
 * kernel's `chain_sketch_edges` hands the sweep, and several tests below rely
 * on it — do not build one of these by hand from unordered edges.
 */
export interface PathPolyline {
  segments: PathSegment[]
  closed: boolean
  /** True when the coordinates came across as f64 (sketch edges). A face loop
   *  arrives as f32 and can never earn an `'ok'` verdict. */
  exact: boolean
}

/** A plane as the kernel stores it: a point on it plus its unit normal. */
export interface PlaneDef {
  point: Vec3
  normal: Vec3
}

/**
 * Why a placement is refused — drives the tool's guidance copy. Neither
 * variant here is fixed by auto-orientation (design §2c): a corner fold-back
 * is refused again after the fold (the fold cannot un-straddle a corner), and
 * a closed path's plane that is ALREADY exactly perpendicular but touches
 * nowhere never triggers the fold in the first place (the kernel only folds
 * on `ProfileNotPerpendicular`, and this is `PathDetachedFromProfile`). A
 * placement merely not-yet-square earns `'orient'` now, not a `RefusalReason`
 * — see the module docs.
 */
export type RefusalReason =
  /** Closed path, sitting on a CORNER and straddling it — decisively on both
   *  sides of the corner along its non-perpendicular flank — the fold
   *  `PathTooTight` refuses (see the module docs' corner-seam paragraph). A
   *  corner is NOT refused merely for being a corner anymore (design §2b),
   *  nor for a profile hanging wholly inside the loop — only this straddle. */
  | 'corner-overhang'
  /** Closed path only: square to a segment, but not touching it anywhere (no
   *  strict-interior crossing, no corner touch). An OPEN path never reaches
   *  this — a detached end is carried to the profile instead (design §2a). */
  | 'detached'

export type StartVerdict =
  | { kind: 'ok'; carried?: boolean }
  /** Square to nothing on the path YET — auto-orientation (design §2c) folds
   *  the profile upright and retries before the kernel would refuse, and that
   *  fold succeeds for essentially any placement near the path. Informational,
   *  not a warning — see the module docs. */
  | { kind: 'orient' }
  | { kind: 'refused'; reason: RefusalReason }
  | { kind: 'unknown' }

// ---------------------------------------------------------------- tolerances

/** The kernel's own tolerances (crates/kernel/src/tol.rs) — used only for the
 *  affirmative `'ok'` verdict on an f64-exact path, where this module's
 *  arithmetic reproduces the kernel's. */
const NORMAL_DIRECTION = 1e-9
const PLANE_DIST = 1e-9
const POINT_MERGE = 1e-9

/** Decisive-refusal slop. A refusal is reported only outside these, which are
 *  ~6 orders of magnitude looser than the kernel tolerances and comfortably
 *  wider than f32 round-trip noise on a face loop — so a `'refused'` verdict
 *  is never an artefact of precision. Everything inside the band is
 *  `'unknown'`: not warned about, and not blessed either. */
const SLOP_NORMAL = 1e-3
const SLOP_DIST = 1e-4

// ------------------------------------------------------------------ vec math

function sub(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
function add(a: Vec3, b: Vec3): Vec3 {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
function scale(a: Vec3, k: number): Vec3 {
  return [a[0] * k, a[1] * k, a[2] * k]
}
function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
function cross(a: Vec3, b: Vec3): Vec3 {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ]
}
function length(a: Vec3): number {
  return Math.sqrt(dot(a, a))
}
/** Unit vector, or null when too short to normalize (kernel `normalized()`). */
function unit(a: Vec3): Vec3 | null {
  const l = length(a)
  if (!(l > 1e-12)) return null
  return [a[0] / l, a[1] / l, a[2] / l]
}
function signedDistance(plane: PlaneDef, p: Vec3): number {
  return dot(plane.normal, sub(p, plane.point))
}

// ------------------------------------------------------------ path structure

/** A path vertex key. Shared vertices come from one kernel vertex record, so
 *  their coordinates are bit-identical and a plain string key is exact. */
function vkey(p: Vec3): string {
  return `${p[0]},${p[1]},${p[2]}`
}


/**
 * Chain `segments` into one walked, consistently ORIENTED path, or null for
 * anything the kernel's own chaining refuses (a branch, or two disjoint
 * components → `PathBranches` / `PathDisconnected`). Returning null is what
 * keeps the cue silent on a path that cannot be swept at all, rather than
 * blessing one component of it.
 *
 * THE ORIENTATION IS THE POINT, not a convenience. A sketch edge stores its
 * own `from`/`to` in whatever order it was drawn, and `sketch_edge_endpoints`
 * hands them over verbatim — two arcs each *ending* at a shared snapped point
 * have that joint as the `b` of BOTH. The kernel never sees that: `Document`'s
 * `chain_sketch_edges` walks the edge graph and emits `path[k] → path[k+1]` in
 * one consistent direction first, so every joint vertex is the START of
 * exactly one segment, which is exactly what lets `from_follow_me` test only
 * `sd_a` when it looks for a facet-joint seam. A mirror that skipped this walk
 * would miss the joint on both sides and report a legal lathe start as
 * `detached`. So the walk is reproduced here, deliberately, before any test
 * runs against the result.
 */
export function chainPath(
  segments: PathSegment[],
): { segments: PathSegment[]; closed: boolean } | null {
  if (segments.length === 0) return null
  const incident = new Map<string, number[]>()
  const at = new Map<string, Vec3>()
  segments.forEach((s, i) => {
    for (const p of [s.a, s.b]) {
      const k = vkey(p)
      at.set(k, p)
      const list = incident.get(k)
      if (list === undefined) incident.set(k, [i])
      else list.push(i)
    }
  })
  const ends: string[] = []
  for (const [k, segs] of incident) {
    if (segs.length === 2) continue
    if (segs.length === 1) ends.push(k)
    else return null // a vertex with three or more edges: the path branches
  }
  if (ends.length !== 0 && ends.length !== 2) return null
  const closed = ends.length === 0

  // Walk from a deterministic start, orienting each segment away from the
  // vertex we arrived at. Visiting every segment exactly once is also the
  // connectivity proof — two disjoint loops pass the degree test above, and
  // the kernel refuses them as PathDisconnected.
  const start = closed ? incident.keys().next().value! : ends[0]
  const visited = new Set<number>()
  const walked: PathSegment[] = []
  let atKey = start
  for (;;) {
    const next = (incident.get(atKey) ?? []).find((i) => !visited.has(i))
    if (next === undefined) break
    visited.add(next)
    const s = segments[next]
    const forward = vkey(s.a) === atKey
    walked.push(forward ? s : { ...s, a: s.b, b: s.a })
    atKey = forward ? vkey(s.b) : vkey(s.a)
  }
  if (visited.size !== segments.length) return null
  return { segments: walked, closed }
}

// ----------------------------------------------------------------- verdict

/** A closed loop's vertices with the curve of the segment LEAVING each. */
interface LoopVertex {
  p: Vec3
  curve: CurveGeom | null
}

/**
 * The kernel's `drop_straight_path_vertices` (ops.rs) on a closed loop: a
 * vertex lying on the segment between its two neighbors, both of its segments
 * plain, is no corner — a face's rim gets one wherever an imprint ends on it.
 * The kernel sweeps through it when keeping it as a station would fold the
 * sweep, so the advance mirror below reads the loop through its real corners
 * only. `tol` is the kernel's POINT_MERGE on an f64 path; an f32 face loop
 * needs a looser one to see the same vertex as straight.
 */
function dropStraight(loop: LoopVertex[], tol: number): LoopVertex[] {
  const out = loop.slice()
  for (;;) {
    const n = out.length
    if (n <= 3) return out
    let hit = -1
    // Vertex 0 is the seam corner the caller walks from: it stays.
    for (let i = 1; i < n && hit < 0; i++) {
      const prev = out[(i + n - 1) % n]
      if (prev.curve !== null || out[i].curve !== null) continue
      const a = prev.p
      const v = out[i].p
      const b = out[(i + 1) % n].p
      const ab = sub(b, a)
      const len2 = dot(ab, ab)
      if (len2 <= tol * tol || length(sub(v, a)) <= tol || length(sub(v, b)) <= tol) continue
      const t = dot(sub(v, a), ab) / len2
      if (t > 0 && t < 1 && length(sub(add(a, scale(ab, t)), v)) <= tol) hit = i
    }
    if (hit < 0) return out
    out.splice(hit, 1)
  }
}

/**
 * The kernel's advance check for an INWARD corner seam (ops.rs, "a corner
 * profile hanging inside the loop"): the ring is carried along the leaving
 * flank onto the corner's miter plane, then transported station by station
 * around the loop exactly as the sweep does. Returns the smallest advance any
 * ring point makes along any segment — the kernel refuses
 * `ProfileTooWideForCorner` at `<= POINT_MERGE` — or null when the transport
 * itself degenerates (a reversal the kernel refuses for its own reasons).
 * `walk` starts at the corner and leaves along the perpendicular flank.
 */
function inwardSeamAdvance(walk: readonly Vec3[], ring: readonly Vec3[]): number | null {
  const m = walk.length
  if (m < 3) return null
  const dirs: Vec3[] = []
  for (let i = 0; i < m; i++) {
    const d = unit(sub(walk[(i + 1) % m], walk[i]))
    if (d === null) return null
    dirs.push(d)
  }
  const station = (s: number): PlaneDef | null => {
    const nrm = unit(add(dirs[(s + m - 1) % m], dirs[s]))
    return nrm === null ? null : { point: walk[s], normal: nrm }
  }
  const carry = (pts: readonly Vec3[], dir: Vec3, onto: PlaneDef): Vec3[] | null => {
    const denom = dot(dir, onto.normal)
    if (Math.abs(denom) < 1e-12) return null
    return pts.map((v) => add(v, scale(dir, -signedDistance(onto, v) / denom)))
  }
  const seam = station(0)
  if (seam === null) return null
  const base = carry(ring, dirs[0], seam)
  if (base === null) return null
  let cur = base
  let minAdv = Infinity
  for (let s = 0; s < m; s++) {
    const target = station((s + 1) % m)
    if (target === null) return null
    const next = carry(cur, dirs[s], target)
    if (next === null) return null
    for (let j = 0; j < cur.length; j++) {
      const adv = dot(sub(next[j], cur[j]), dirs[s])
      if (adv < minAdv) minAdv = adv
    }
    cur = next
  }
  return minAdv
}

/** How a corner seam reads for the hovered profile (design §2b). */
interface CornerReading {
  /** The kernel accepts this corner as the seam, at its own tolerances. */
  exactOk: boolean
  /** Decisively refused at this corner, and why: the profile straddles the
   *  corner (`PathTooTight`), or hangs inside the loop but is too wide for it
   *  (`ProfileTooWideForCorner`). Null when it is not decisive. */
  refuse: 'overhang' | 'too-wide' | null
}

/**
 * The kernel's corner-seam rule (design §2b), reduced to the profile plane.
 * `d` is the OTHER (non-perpendicular) flank's direction, pointing INTO the
 * corner, and `(point − corner) · d` says which side of the corner each
 * profile point is on:
 *
 *   - every point at or BEYOND the corner (`≥ −POINT_MERGE`): the outward
 *     seam, the wedge closes it — legal;
 *   - every point at or BEFORE it (`≤ POINT_MERGE`), some decisively before:
 *     the profile hangs INSIDE the loop (a rebate drawn at a corner of the
 *     face it runs around). The kernel carries it onto the corner's miter
 *     plane and sweeps it like the mid-edge placement — legal unless it is
 *     too wide for the loop (`ProfileTooWideForCorner`), which the transport
 *     mirror `inwardSeamAdvance` decides;
 *   - points on BOTH sides: the profile straddles the corner and folds into
 *     its own material — `PathTooTight`.
 *
 * `walk` is the loop from the corner, leaving along the perpendicular flank.
 * `ring` is the hovered profile's boundary points — without it (no geometry
 * in hand yet) neither an `ok` nor a `refused` can be earned, only silence.
 */
function cornerFold(
  corner: Vec3,
  farOfOtherFlank: Vec3,
  walk: readonly Vec3[],
  ring: readonly Vec3[] | undefined,
): CornerReading {
  const d = unit(sub(corner, farOfOtherFlank))
  if (d === null || ring === undefined || ring.length === 0) {
    return { exactOk: false, refuse: null }
  }
  let minVal = Infinity
  let maxVal = -Infinity
  for (const p of ring) {
    const v = dot(sub(p, corner), d)
    if (v < minVal) minVal = v
    if (v > maxVal) maxVal = v
  }
  if (minVal >= -POINT_MERGE) return { exactOk: true, refuse: null }
  if (maxVal > SLOP_DIST) {
    // Points beyond the corner, and some behind it: straddling.
    return { exactOk: false, refuse: minVal < -SLOP_DIST ? 'overhang' : null }
  }
  // Inside the loop (or within the slop band of it): the transport decides.
  const adv = inwardSeamAdvance(walk, ring)
  if (adv === null) return { exactOk: false, refuse: null }
  return {
    exactOk: maxVal <= POINT_MERGE && adv > POINT_MERGE,
    refuse: minVal < -SLOP_DIST && adv < -SLOP_DIST ? 'too-wide' : null,
  }
}

/**
 * Would the kernel accept a profile on `plane` as a start for `path`?
 *
 * This mirrors the anchor scan in `Object::from_follow_me`. It is
 * three-valued on purpose — see the module docs for exactly what each answer
 * is allowed to claim.
 *
 * `profileRing` is the hovered profile's own boundary points (world space,
 * any winding) — needed ONLY to judge a closed-path CORNER seam's fold test
 * (design §2b); every other branch ignores it. Omit it and a corner
 * placement reads `'unknown'` rather than ever guessing.
 */
export function evaluateStart(
  path: PathPolyline,
  plane: PlaneDef,
  profileRing?: readonly Vec3[],
): StartVerdict {
  if (path.segments.length === 0) return { kind: 'unknown' }
  return path.closed ? evaluateClosed(path, plane, profileRing) : evaluateOpen(path, plane)
}

function evaluateClosed(
  path: PathPolyline,
  plane: PlaneDef,
  profileRing: readonly Vec3[] | undefined,
): StartVerdict {
  const n = plane.normal
  const m = path.segments.length

  // Exact (kernel-tolerance) tallies, and decisive (slop-band) ones. The two
  // scans run together because they differ only in the thresholds applied.
  let exactCandidate = false
  let exactAnyPerp = false
  let maybeSquare = false // some segment is square within the slop band
  let maybeCandidate = false // ...and might cross that segment's interior
  let maybeCornerAmbiguous = false // ...sits near an endpoint, fold undecided
  // Corners decisively refused, with how far each is from the profile: the
  // kernel ranks its seam candidates nearest the profile first and reports
  // the NEAREST one's refusal when every candidate refuses.
  const cornerRefusals: { reason: 'overhang' | 'too-wide'; dist: number }[] = []
  const centroid: Vec3 | null =
    profileRing === undefined || profileRing.length === 0
      ? null
      : scale(
          profileRing.reduce<Vec3>((acc, p) => add(acc, p), [0, 0, 0]),
          1 / profileRing.length,
        )
  // The loop from vertex `start`, leaving along segment `start` (forward) or
  // against segment `start - 1` (reversed), straight vertices dropped.
  const straightTol = path.exact ? POINT_MERGE : 1e-5
  const walkFrom = (start: number, forward: boolean): Vec3[] => {
    const loop: LoopVertex[] = []
    for (let i = 0; i < m; i++) {
      if (forward) {
        const s = path.segments[(start + i) % m]
        loop.push({ p: s.a, curve: s.curve })
      } else {
        const j = (start - i + m) % m
        loop.push({ p: path.segments[j].a, curve: path.segments[(j + m - 1) % m].curve })
      }
    }
    return dropStraight(loop, straightTol).map((v) => v.p)
  }

  for (let k = 0; k < path.segments.length; k++) {
    const seg = path.segments[k]
    const dir = unit(sub(seg.b, seg.a))
    if (dir === null) continue
    const sdA = signedDistance(plane, seg.a)
    const sdB = signedDistance(plane, seg.b)
    const g = seg.curve

    if (g !== null) {
      // Curve-attributed facet: perpendicularity is the RADIAL test against
      // the analytic circle, not the chord (ops.rs "the profile plane is
      // perpendicular to the circle somewhere iff it is RADIAL").
      const curveNormal = unit(cross(dir, sub(seg.a, g.center)))
      if (curveNormal === null) continue
      const offAxis = Math.abs(dot(n, curveNormal))
      const offCenter = Math.abs(signedDistance(plane, g.center))
      if (offAxis > SLOP_NORMAL || offCenter > SLOP_DIST) continue // decisively not radial
      maybeSquare = true
      const radial = offAxis <= NORMAL_DIRECTION && offCenter <= PLANE_DIST
      if (radial) exactAnyPerp = true

      if (Math.abs(sdA) <= POINT_MERGE) {
        // The plane passes through this facet's start vertex. Legal only when
        // the incoming facet belongs to the SAME curve chain (then the joint's
        // miter plane IS the profile plane) and the joint tangent is the
        // profile normal. `prev` is the chain predecessor — testing only the
        // segment START, as the kernel does, is sound BECAUSE `chainPath` has
        // already oriented the walk, so every vertex is some segment's start.
        const prev = path.segments[(k + m - 1) % m]
        if (sameCurve(prev.curve, g)) {
          const dPrev = unit(sub(seg.a, prev.a))
          if (dPrev !== null) {
            const bisector = unit(add(dPrev, dir))
            if (bisector !== null) {
              const align = Math.abs(dot(n, bisector))
              if (align >= 1 - SLOP_NORMAL) maybeCandidate = true
              if (radial && align >= 1 - NORMAL_DIRECTION) exactCandidate = true
            }
          }
        }
      } else if (Math.abs(sdB) > POINT_MERGE && sdA * sdB < 0) {
        maybeCandidate = true
        if (radial) exactCandidate = true
      }
      continue
    }

    // Plain segment: the chord test.
    const align = Math.abs(dot(n, dir))
    if (align < 1 - SLOP_NORMAL) continue // decisively not square to this segment
    maybeSquare = true
    const square = align >= 1 - NORMAL_DIRECTION
    if (square) exactAnyPerp = true
    // Near (or exactly on) one of this segment's endpoints. This could be a
    // genuine interior crossing that merely lands close to the vertex — still
    // unconditionally legal, exactly as a mid-run crossing is — or a genuine
    // CORNER touch, now itself conditionally legal via the fold test (design
    // §2b) instead of an automatic refusal.
    if (Math.abs(sdA) <= SLOP_DIST || Math.abs(sdB) <= SLOP_DIST) {
      if (square && Math.abs(sdA) > POINT_MERGE && Math.abs(sdB) > POINT_MERGE && sdA * sdB < 0) {
        exactCandidate = true
        maybeCandidate = true
        continue
      }
      // A corner touch: `atA` picks which endpoint (mirroring the kernel's
      // own sd_a-then-sd_b order), and the OTHER flank at that vertex — the
      // one whose direction the fold test is measured against — is the
      // chain predecessor when the touch is at `a`, the chain successor when
      // it is at `b` (each necessarily shares that vertex, by `chainPath`'s
      // orientation invariant).
      const atA = Math.abs(sdA) <= Math.abs(sdB)
      const corner = atA ? seg.a : seg.b
      const farOther = atA
        ? path.segments[(k + m - 1) % m].a
        : path.segments[(k + 1) % m].b
      const exactTouch = atA ? Math.abs(sdA) <= POINT_MERGE : Math.abs(sdB) <= POINT_MERGE
      // Leaving along this segment: forward from `a`, or backward from `b`.
      const walk = atA ? walkFrom(k, true) : walkFrom((k + 1) % m, false)
      const fold = cornerFold(corner, farOther, walk, profileRing)
      if (square && exactTouch && path.exact && fold.exactOk) {
        exactCandidate = true
      } else if (fold.refuse !== null && centroid !== null) {
        cornerRefusals.push({ reason: fold.refuse, dist: length(sub(corner, centroid)) })
      } else {
        maybeCornerAmbiguous = true
      }
      continue
    }
    if (sdA * sdB > 0) continue // both on one side: no crossing here
    maybeCandidate = true
    if (square && Math.abs(sdA) > POINT_MERGE && Math.abs(sdB) > POINT_MERGE && sdA * sdB < 0) {
      exactCandidate = true
    }
  }

  if (path.exact && exactCandidate) return { kind: 'ok' }
  // Might be accepted (an ordinary interior-crossing ambiguity, or a corner
  // whose fold test is itself too close to call) — never warn either way.
  if (maybeCandidate || maybeCornerAmbiguous) return { kind: 'unknown' }
  if (!maybeSquare) {
    // Square to nothing — this used to be the decisive `ProfileNotPerpendicular`
    // refusal; auto-orientation (design §2c) now folds the profile upright and
    // retries before the kernel would ever refuse for this reason, and that
    // fold succeeds for essentially any placement near the path (a flat profile
    // beside a circle, a molding profile outside a frame — see `ops.rs`'s specs
    // for `orient_profile_to_path`). Informational, not a warning.
    return { kind: 'orient' }
  }
  // Square, sitting on a corner, and every corner decisively refused. The
  // kernel reports the refusal of the corner NEAREST the profile: straddling
  // it is the fold `PathTooTight` (predictable from the profile's own extent);
  // hanging inside the loop but too wide for it is `ProfileTooWideForCorner`,
  // which has no hover copy — say nothing rather than the wrong thing.
  if (cornerRefusals.length > 0) {
    const nearest = Math.min(...cornerRefusals.map((r) => r.dist))
    const reasons = new Set(
      cornerRefusals.filter((r) => r.dist <= nearest + SLOP_DIST).map((r) => r.reason),
    )
    if (reasons.size === 1 && reasons.has('overhang')) {
      return { kind: 'refused', reason: 'corner-overhang' }
    }
    return { kind: 'unknown' }
  }
  // Only claim "detached" when the exact scan agrees a segment really is
  // perpendicular; otherwise the slop band alone is too weak a basis.
  if (path.exact && exactAnyPerp) return { kind: 'refused', reason: 'detached' }
  return { kind: 'unknown' }
}

/**
 * An open path's start rule (design §2a): the profile need only be
 * perpendicular to ONE end's leave direction — the matching end vertex need
 * NOT sit on the profile plane at all. A detached-but-perpendicular end is
 * carried rigidly to the profile, so this never refuses for detachment; it
 * only ever refuses when the profile is square to NEITHER end.
 */
function evaluateOpen(path: PathPolyline, plane: PlaneDef): StartVerdict {
  const n = plane.normal
  const m = path.segments.length
  let maybeSquare = false
  let exactSquare = false
  // Whether some qualifying (exactly square) end ALSO sits on the profile
  // plane already — the kernel attaches directly there rather than carrying
  // (it checks `path[0]` first, then `path[len-1]`, but for the app-side
  // verdict — which only needs to say whether SOME attach-without-carry
  // exists — an unordered OR is sufficient; it never changes ok/refused).
  let exactOnPlane = false

  // The two ends of the walked chain — the kernel's `path[0]`/`path[len-1]`,
  // each measured against the segment leaving it.
  const ends: { end: Vec3; far: Vec3; seg: PathSegment }[] = [
    { end: path.segments[0].a, far: path.segments[0].b, seg: path.segments[0] },
    { end: path.segments[m - 1].b, far: path.segments[m - 1].a, seg: path.segments[m - 1] },
  ]
  for (const { end, far, seg } of ends) {
    const chord = unit(sub(far, end))
    if (chord === null) continue
    // An attributed end segment measures against the drawn curve's TANGENT at
    // the end vertex; a plain segment keeps the chord test (ops.rs `end_perp`).
    let align: number
    if (seg.curve !== null) {
      const radial = unit(sub(end, seg.curve.center))
      if (radial === null) continue
      const tangent = unit(sub(chord, scale(radial, dot(chord, radial))))
      if (tangent === null) continue
      align = Math.abs(dot(n, tangent))
    } else {
      align = Math.abs(dot(n, chord))
    }
    if (align < 1 - SLOP_NORMAL) continue
    maybeSquare = true
    if (align >= 1 - NORMAL_DIRECTION) {
      exactSquare = true
      if (Math.abs(signedDistance(plane, end)) <= PLANE_DIST) exactOnPlane = true
    }
  }

  if (path.exact && exactSquare) return { kind: 'ok', carried: !exactOnPlane }
  if (maybeSquare) return { kind: 'unknown' }
  // Square to neither end — used to be the decisive refusal; auto-orientation
  // (design §2c) now folds the profile onto whichever end it is nearest and
  // retries (see `follow_me_auto_orient_hinges_at_a_touching_flap` and
  // `follow_me_folds_a_parallel_profile_upright` in `op_specs.rs`), so this is
  // informational now, not a warning.
  return { kind: 'orient' }
}

/** Plain-language guidance for a hover-time refusal — what the user must do,
 *  not what the kernel called it. */
export function refusalGuidance(reason: RefusalReason): string {
  switch (reason) {
    case 'corner-overhang':
      return t`This profile will be refused: it hangs back over the corner, folding into its own material. Slide it fully past the corner.`
    case 'detached':
      return t`This profile will be refused: it is square to the path but does not touch it. Move it onto the path.`
  }
}

/** Plain-language guidance for the `'orient'` verdict — informational, not a
 *  warning: the profile isn't square to the path yet, but Follow Me will
 *  stand it upright automatically before sweeping (design §2c). */
export function orientGuidance(): string {
  return t`This profile isn’t square to the path yet — Follow Me will stand it upright automatically before sweeping.`
}
