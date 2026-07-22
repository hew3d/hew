/**
 * followMeStart — where a Follow Me profile may legally START on a path, and
 * whether the profile the cursor is over would be refused.
 *
 * WHY THIS EXISTS. Follow Me's start rule is invisible: the kernel accepts a
 * profile only when the profile's PLANE meets the path in a very particular
 * way, and every other placement comes back as a post-click
 * `[ProfileNotPerpendicular]` / `[PathDetachedFromProfile]` toast. This module
 * is the app-side mirror of that rule, so the tool can mark the legal starts
 * on the path and warn while the user is still hovering.
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
 *   containing the circle's axis — and the ones a user can actually reach by
 *   drawing on an axis-aligned plane are the four QUADRANTS. That is what the
 *   tool marks.
 *
 *   Closed path, plain segment. The plane's normal must be parallel to the
 *   segment (|n·dir| ≥ 1 − NORMAL_DIRECTION) AND the plane must either cross
 *   the segment's STRICT INTERIOR, or pass through one of its two endpoints —
 *   a CORNER SEAM (design §2b). A corner seam is legal exactly when every
 *   point of the hovered profile sits BEYOND the corner along the OTHER
 *   (non-perpendicular) flank's own direction — `(point − corner) · d ≥
 *   −POINT_MERGE`, `d` being that flank's unit direction pointing INTO the
 *   corner. A profile hanging back over that flank folds into its own swept
 *   material and refuses `PathTooTight`, never nudged. This is only decidable
 *   with the actual profile geometry in hand, which is why `evaluateStart`
 *   takes an optional `profileRing` — without one, a corner placement can
 *   only ever read `'unknown'`.
 *
 *   Closed path, facet joint of ONE drawn curve. A vertex shared by two
 *   segments of the SAME curve chain IS a legal start (the joint's miter plane
 *   is the profile plane) — unconditionally, no fold test, since the joint's
 *   own tangent already IS the miter plane. Combined with the corner-seam
 *   capability above, `startAffordance` now marks almost every closed-path
 *   vertex as a POTENTIAL start (position-only — see its own doc comment):
 *   the only vertex left permanently blocked is a joint between two
 *   DIFFERENT drawn curves, which no kernel anchor covers at all.
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
 * refuses is worse than no cue, so the verdict is deliberately three-valued:
 *
 *   - `'ok'` is claimed ONLY for f64-exact paths (sketch edges, whose
 *     coordinates cross the WASM boundary as f64 and so reproduce the kernel's
 *     arithmetic) and only under the kernel's own tolerances. A face-loop path
 *     arrives as f32 (`face_boundary`), far coarser than the 1e-9 tolerances,
 *     so `'ok'` is never claimed for one.
 *   - `'refused'` is claimed only when the placement is DECISIVELY wrong —
 *     outside a slop band far wider than either the kernel tolerance or f32
 *     noise. A placement that is merely a hair off reports `'unknown'`.
 *   - `'unknown'` is the honest answer in the band between, and for anything
 *     this module cannot chain (a branching or disconnected path).
 *
 * And even `'ok'` means only "the START is valid". The sweep can still refuse
 * for reasons that depend on the whole transport — `PathReverses`, the closed
 * seam's exact re-landing check — that are not decidable from the start
 * placement alone. The tool's copy says "ready to sweep", never "this will
 * work". (A corner seam's own fold refusal, `PathTooTight`, IS decidable from
 * the start placement — see the corner-seam paragraph above — and is the one
 * exception this module predicts.)
 */

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

/** Markers the tool draws on the picked path. */
export interface StartAffordance {
  /** Points a profile may start at — the four quadrants of a circle path, or
   *  the two ends of an open path. */
  legal: Vec3[]
  /** Path vertices a profile may NOT start at (real corners). */
  blocked: Vec3[]
}

/** Why a placement is refused — drives the tool's guidance copy. */
export type RefusalReason =
  /** Circle path: the profile's plane does not contain the circle's axis. */
  | 'not-radial'
  /** The profile plane is square to no part of the path. */
  | 'not-square'
  /** Closed path, sitting on a CORNER, decisively hanging back over the
   *  corner's non-perpendicular flank — the fold `PathTooTight` refuses (see
   *  the module docs' corner-seam paragraph). A corner is NOT refused merely
   *  for being a corner anymore (design §2b) — only this overhang is. */
  | 'corner-overhang'
  /** Closed path only: square to a segment, but not touching it anywhere (no
   *  strict-interior crossing, no corner touch). An OPEN path never reaches
   *  this — a detached end is carried to the profile instead (design §2a). */
  | 'detached'

export type StartVerdict =
  | { kind: 'ok'; carried?: boolean }
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

/**
 * The kernel's `geom2d::plane_axes` — the deterministic in-plane basis a
 * sketch's analytic rims are expressed in. Reproduced exactly so the quadrant
 * markers land on the SAME points the inference engine offers as
 * `SnapKind::Quadrant` snaps (which carry extra snap gravity), rather than on
 * a differently-phased set the cursor would not stick to.
 */
export function planeAxes(normal: Vec3): { u: Vec3; v: Vec3 } | null {
  const [nx, ny, nz] = [Math.abs(normal[0]), Math.abs(normal[1]), Math.abs(normal[2])]
  const reference: Vec3 = nx <= ny && nx <= nz ? [1, 0, 0] : ny <= nz ? [0, 1, 0] : [0, 0, 1]
  const u = unit(cross(normal, reference))
  if (u === null) return null
  const v = unit(cross(normal, u))
  if (v === null) return null
  return { u, v }
}

// ------------------------------------------------------------ path structure

/** A path vertex key. Shared vertices come from one kernel vertex record, so
 *  their coordinates are bit-identical and a plain string key is exact. */
function vkey(p: Vec3): string {
  return `${p[0]},${p[1]},${p[2]}`
}


/**
 * Whether the path is one closed loop of segments all attributed to the SAME
 * analytic circle — the kernel's `circular_path_axis` precondition. Returns
 * the revolution axis (center + the loop's plane normal, supplied by the
 * caller from the path sketch's plane) or null.
 */
export function circlePath(
  path: PathPolyline,
  sketchNormal: Vec3,
): { center: Vec3; axis: Vec3; radius: number } | null {
  if (!path.closed || path.segments.length < 3) return null
  const first = path.segments[0].curve
  if (first === null || !Number.isFinite(first.radius) || first.radius <= POINT_MERGE) return null
  for (const s of path.segments) {
    const g = s.curve
    if (g === null) return null
    if (length(sub(g.center, first.center)) > POINT_MERGE) return null
    if (Math.abs(g.radius - first.radius) > POINT_MERGE) return null
  }
  const axis = unit(sketchNormal)
  if (axis === null) return null
  return { center: first.center, axis, radius: first.radius }
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

/**
 * The markers to draw on a picked path.
 *
 * - A closed circle path → its four quadrant points, phased exactly as the
 *   inference engine's `Quadrant` snaps are (see `planeAxes`).
 * - Any other closed path → every vertex is now a POTENTIAL corner-seam start
 *   (design §2b), EXCEPT one where both adjacent flanks are curve-attributed
 *   and belong to different curves — the kernel's `Anchor::Corner` candidate
 *   only ever comes from a PLAIN (non-curve) flank being perpendicular and
 *   touching the vertex (`ops.rs`: the corner branch lives inside the
 *   plain-segment scan only), so a vertex with at least one plain flank, or
 *   with two flanks of the SAME curve (a facet joint), can always be reached
 *   by *some* profile placement and is marked legal; a joint between two
 *   DIFFERENT curves has no kernel anchor at all and stays blocked. This is a
 *   POSITION-only marker, exactly like the quadrants/ends below — it says
 *   "you can aim here", not "any orientation here works"; the per-hover
 *   verdict (`evaluateStart`) still judges the actual profile.
 * - An open path → its two ends, the only points a sweep can start from.
 */
export function startAffordance(path: PathPolyline, sketchNormal: Vec3 | null): StartAffordance {
  const legal: Vec3[] = []
  const blocked: Vec3[] = []
  if (path.segments.length === 0) return { legal, blocked }

  if (sketchNormal !== null) {
    const circle = circlePath(path, sketchNormal)
    if (circle !== null) {
      const basis = planeAxes(circle.axis)
      if (basis !== null) {
        for (const q of [0, 0.5, 1, 1.5]) {
          const a = q * Math.PI
          legal.push(
            add(
              circle.center,
              add(scale(basis.u, circle.radius * Math.cos(a)), scale(basis.v, circle.radius * Math.sin(a))),
            ),
          )
        }
      }
      return { legal, blocked }
    }
  }

  const m = path.segments.length
  if (!path.closed) {
    // An OPEN path can only be swept from an end, so the ends are the whole
    // story — and no vertex gets a blocked marker, because an interior vertex
    // is no more illegal than the straight run beside it. Marking only the
    // vertices would wrongly imply the runs were fair game.
    legal.push(path.segments[0].a, path.segments[m - 1].b)
    return { legal, blocked }
  }
  for (let k = 0; k < m; k++) {
    // The joint at this segment's start, between it and its chain predecessor.
    const prev = path.segments[(k + m - 1) % m]
    const cur = path.segments[k]
    // Same drawn curve on both sides → a facet joint, which the kernel accepts
    // as a seam (its miter plane IS the profile plane) — legal regardless of
    // where the profile stands. Otherwise, a vertex with at least one PLAIN
    // flank is a potential corner seam (design §2b) — legal, contingent on
    // the profile's own placement (judged per-hover by `evaluateStart`).
    // Only a joint between two DIFFERENT curves has no anchor at all.
    if (sameCurve(prev.curve, cur.curve) || prev.curve === null || cur.curve === null) {
      legal.push(cur.a)
      continue
    }
    blocked.push(cur.a)
  }
  return { legal, blocked }
}

// ----------------------------------------------------------------- verdict

/**
 * The kernel's per-ring-vertex advance check across a corner-seam wedge
 * (design §2b), reduced to the profile plane: legal exactly when every point
 * of the profile sits BEYOND the corner along `d` — the OTHER (non-
 * perpendicular) flank's own direction, pointing INTO the corner (i.e. from
 * that flank's far endpoint toward the corner). `(point − corner) · d ≥
 * −POINT_MERGE` is legal; decisively less is the fold `PathTooTight`.
 *
 * `ring` is the hovered profile's boundary points — without it (no geometry
 * in hand yet) neither an `ok` nor a `refused` can be earned, only silence.
 */
function cornerFold(
  corner: Vec3,
  farOfOtherFlank: Vec3,
  ring: readonly Vec3[] | undefined,
): { exactOk: boolean; decisiveRefuse: boolean } {
  const d = unit(sub(corner, farOfOtherFlank))
  if (d === null || ring === undefined || ring.length === 0) {
    return { exactOk: false, decisiveRefuse: false }
  }
  let minVal = Infinity
  for (const p of ring) {
    const v = dot(sub(p, corner), d)
    if (v < minVal) minVal = v
  }
  return { exactOk: minVal >= -POINT_MERGE, decisiveRefuse: minVal < -SLOP_DIST }
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
  let decisiveCornerOverhang = false // ...sits on a corner and folds back over it

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
      const fold = cornerFold(corner, farOther, profileRing)
      if (square && exactTouch && path.exact && fold.exactOk) {
        exactCandidate = true
      } else if (fold.decisiveRefuse) {
        decisiveCornerOverhang = true
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
    return { kind: 'refused', reason: circleLike(path) ? 'not-radial' : 'not-square' }
  }
  // Square, sitting on a corner, and decisively folded back over the
  // non-perpendicular flank — the refusal the kernel's advance check makes
  // (`PathTooTight`), predictable here because it only needs the profile's
  // own extent, not the whole transport.
  if (decisiveCornerOverhang) return { kind: 'refused', reason: 'corner-overhang' }
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
  return { kind: 'refused', reason: 'not-square' }
}

/** Whether every segment carries an analytic circle — used only to pick the
 *  right wording for a "not square anywhere" refusal. */
function circleLike(path: PathPolyline): boolean {
  return path.segments.length > 0 && path.segments.every((s) => s.curve !== null)
}

/** Plain-language guidance for a hover-time refusal — what the user must do,
 *  not what the kernel called it. */
export function refusalGuidance(reason: RefusalReason): string {
  switch (reason) {
    case 'not-radial':
      return 'This profile will be refused: it must stand on a plane through the circle’s centre. Move it onto a marked quadrant.'
    case 'not-square':
      return 'This profile will be refused: it is not square to the path. Stand it across the path where the sweep should start.'
    case 'corner-overhang':
      return 'This profile will be refused: it hangs back over the corner, folding into its own material. Slide it fully past the corner.'
    case 'detached':
      return 'This profile will be refused: it is square to the path but does not touch it. Move it onto the path.'
  }
}
