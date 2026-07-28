/**
 * flatten — pure glyph-outline geometry for 3D Text (docs/design/3d-text.md).
 *
 * No three.js, DOM, or wasm imports — fully testable in Node/vitest, mirroring
 * the arcMath.ts / moveInput.ts convention. Everything here operates in the
 * font's own em-square coordinates (opentype.js's `Path.commands`, Y
 * increasing DOWNWARD — canvas convention) until a caller maps the result
 * into a world-space plane.
 *
 * Pipeline: `pathCommandsToContours` walks a `Path`'s M/L/C/Q/Z commands,
 * flattening each Bézier adaptively (`flattenCubic`/`flattenQuad`) by the
 * SAME sagitta budget the drawn-curves precedent uses
 * (`tools/arcMath.ts`'s `DRAW_SAGITTA_TOL_M`, ~0.5 mm), scaled into
 * em-units for the glyph's placed height (`sagittaToleranceEm`) so a 10 mm
 * letter and a 500 mm sign both facet cleanly. `weldContour` drops
 * duplicate consecutive points (the sketch's own weld tolerance) and
 * degenerate/zero-area contours. `classifyContourFill` decides, for each
 * contour, whether it is glyph MATERIAL or a counter's own bare interior —
 * the selection a caller must make before extruding (see the doc comment
 * on `kernel::Document::place_text`): an 'O' traces as two nested loops,
 * and blindly extruding both would fill the letter's hole solid.
 */

/** 2D point in whatever space the caller is working in (em-units or world). */
export type Pt2 = readonly [number, number]

// ─────────────────────────────────────────────────────── flattening tolerance

/** The drawn-curves sagitta budget (`tools/arcMath.ts`'s `DRAW_SAGITTA_TOL_M`)
 *  — half a millimeter, an FDM printer's practical tolerance and below
 *  silhouette visibility. Glyph curves carry no analytic surface class (a
 *  facet is the final geometry, never re-faceted on export), so this is the
 *  one and only accuracy budget a placed letter's curves get. */
export const GLYPH_SAGITTA_TOL_M = 5e-4

/** Floor/cap on how many flattening subdivisions one Bézier segment may take
 *  — mirrors `arcMath.ts`'s `MIN/MAX_SEGMENTS_PER_TURN` floor-and-cap shape,
 *  bounding cost for a degenerate (near-zero-radius) curve on one end and
 *  megabyte meshes on the other. Expressed as a recursion-depth cap (each
 *  level doubles the segment count), not a segment-count cap directly —
 *  the recursive subdivision below naturally halves the sagitta error per
 *  level, so a depth cap bounds both facet count and worst-case error the
 *  same way `MAX_SEGMENTS_PER_TURN` bounds a circle's facet count.
 */
export const MAX_FLATTEN_DEPTH = 10

/**
 * The Bézier-flattening tolerance in the glyph's OWN em-square units, for a
 * glyph placed at `heightMeters` tall in a font whose em-square is
 * `unitsPerEm` units. `GLYPH_SAGITTA_TOL_M` is a WORLD-space (meters)
 * budget; flattening happens before the em→world scale is applied, so the
 * budget is converted the other way: `tolEm = tolWorld / (heightMeters /
 * unitsPerEm)`. Degenerate `heightMeters` (≤ 0 or non-finite) falls back to
 * treating the em-square as if it were placed at `unitsPerEm` meters tall
 * (tolEm = tolWorld·unitsPerEm/unitsPerEm = tolWorld) — coarse but finite,
 * never a divide-by-zero NaN cascade into the flattener.
 */
export function sagittaToleranceEm(heightMeters: number, unitsPerEm: number): number {
  if (!Number.isFinite(heightMeters) || heightMeters <= 0) {
    return GLYPH_SAGITTA_TOL_M
  }
  return (GLYPH_SAGITTA_TOL_M * unitsPerEm) / heightMeters
}

// ─────────────────────────────────────────────────────────── Bézier flattening

/** Perpendicular distance from `p` to the line through `a`→`b` (0 if `a===b`). */
function distToLine(p: Pt2, a: Pt2, b: Pt2): number {
  const dx = b[0] - a[0]
  const dy = b[1] - a[1]
  const len = Math.hypot(dx, dy)
  if (len < 1e-12) return Math.hypot(p[0] - a[0], p[1] - a[1])
  // |cross(b-a, p-a)| / |b-a|
  return Math.abs(dx * (p[1] - a[1]) - dy * (p[0] - a[0])) / len
}

function lerp(a: Pt2, b: Pt2, t: number): Pt2 {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

function quadMid(p0: Pt2, p1: Pt2, p2: Pt2, t: number): Pt2 {
  const a = lerp(p0, p1, t)
  const b = lerp(p1, p2, t)
  return lerp(a, b, t)
}

function cubicMid(p0: Pt2, p1: Pt2, p2: Pt2, p3: Pt2, t: number): Pt2 {
  const a = lerp(p0, p1, t)
  const b = lerp(p1, p2, t)
  const c = lerp(p2, p3, t)
  const ab = lerp(a, b, t)
  const bc = lerp(b, c, t)
  return lerp(ab, bc, t)
}

/**
 * Adaptively flattens a quadratic Bézier `p0,p1,p2` into a polyline whose
 * worst deviation from the true curve is within `tolEm`. Returns points
 * EXCLUDING `p0` (so consecutive segments concatenate without a duplicate
 * join point) and INCLUDING `p2`. The flatness test is the control point's
 * distance from the chord `p0→p2` — the standard de Casteljau recursive
 * flattening criterion; a straight (collinear) "curve" returns just `[p2]`.
 */
export function flattenQuad(p0: Pt2, p1: Pt2, p2: Pt2, tolEm: number, depth = 0): Pt2[] {
  const flat = distToLine(p1, p0, p2) <= tolEm
  if (flat || depth >= MAX_FLATTEN_DEPTH) {
    return [p2]
  }
  const mid = quadMid(p0, p1, p2, 0.5)
  const leftP1 = lerp(p0, p1, 0.5)
  const rightP1 = lerp(p1, p2, 0.5)
  const left = flattenQuad(p0, leftP1, mid, tolEm, depth + 1)
  const right = flattenQuad(mid, rightP1, p2, tolEm, depth + 1)
  return [...left, ...right]
}

/**
 * Adaptively flattens a cubic Bézier `p0,p1,p2,p3` into a polyline within
 * `tolEm` of the true curve — the flatness test is the WORSE of the two
 * control points' distances from the chord `p0→p3` (either alone can bulge
 * independently, an S-curve's classic failure mode for a single-control-
 * point test). Same exclude-`p0`/include-`p3` contract as `flattenQuad`.
 */
export function flattenCubic(
  p0: Pt2,
  p1: Pt2,
  p2: Pt2,
  p3: Pt2,
  tolEm: number,
  depth = 0,
): Pt2[] {
  const dev = Math.max(distToLine(p1, p0, p3), distToLine(p2, p0, p3))
  if (dev <= tolEm || depth >= MAX_FLATTEN_DEPTH) {
    return [p3]
  }
  const p01 = lerp(p0, p1, 0.5)
  const p12 = lerp(p1, p2, 0.5)
  const p23 = lerp(p2, p3, 0.5)
  const p012 = lerp(p01, p12, 0.5)
  const p123 = lerp(p12, p23, 0.5)
  const mid = cubicMid(p0, p1, p2, p3, 0.5)
  const left = flattenCubic(p0, p01, p012, mid, tolEm, depth + 1)
  const right = flattenCubic(mid, p123, p23, p3, tolEm, depth + 1)
  return [...left, ...right]
}

// ─────────────────────────────────────────────────────────── path → contours

/** The subset of opentype.js's `PathCommand` this module consumes — kept
 *  narrow and structural so this module never imports opentype.js itself
 *  (pure-geometry, dependency-free). */
export type FlattenCommand =
  | { type: 'M'; x: number; y: number }
  | { type: 'L'; x: number; y: number }
  | { type: 'C'; x1: number; y1: number; x2: number; y2: number; x: number; y: number }
  | { type: 'Q'; x1: number; y1: number; x: number; y: number }
  | { type: 'Z' }

/**
 * Walks a flattened command stream (an opentype.js `Path.commands`,
 * possibly spanning many glyphs — `Font.getPath` already lays out a whole
 * run with advance/kerning applied) into closed polygon contours, each
 * Bézier flattened adaptively per `tolEm`. Every contour is implicitly
 * closed (an `M` starts a new one; a `Z` — or the next `M`/end of stream —
 * closes it back to that `M`'s point), matching how glyph outlines are
 * always closed loops even when a font's `Z` command is elided. Degenerate
 * (fewer than 3 points) contours are dropped.
 */
export function pathCommandsToContours(commands: readonly FlattenCommand[], tolEm: number): Pt2[][] {
  const contours: Pt2[][] = []
  let current: Pt2[] = []
  let start: Pt2 | null = null
  let cursor: Pt2 = [0, 0]

  const closeCurrent = () => {
    if (current.length >= 3) contours.push(current)
    current = []
    start = null
  }

  for (const cmd of commands) {
    switch (cmd.type) {
      case 'M':
        closeCurrent()
        cursor = [cmd.x, cmd.y]
        start = cursor
        current = [cursor]
        break
      case 'L':
        cursor = [cmd.x, cmd.y]
        current.push(cursor)
        break
      case 'Q': {
        const p1: Pt2 = [cmd.x1, cmd.y1]
        const p2: Pt2 = [cmd.x, cmd.y]
        current.push(...flattenQuad(cursor, p1, p2, tolEm))
        cursor = p2
        break
      }
      case 'C': {
        const p1: Pt2 = [cmd.x1, cmd.y1]
        const p2: Pt2 = [cmd.x2, cmd.y2]
        const p3: Pt2 = [cmd.x, cmd.y]
        current.push(...flattenCubic(cursor, p1, p2, p3, tolEm))
        cursor = p3
        break
      }
      case 'Z':
        if (start !== null) cursor = start
        closeCurrent()
        break
    }
  }
  closeCurrent()
  return contours
}

// ───────────────────────────────────────────────────────────────── welding

/** Mirrors `kernel::tol::POINT_MERGE` (1e-9 world units) — the sketch's own
 *  weld tolerance for "these two points are the same point". Used here in
 *  EM-SQUARE units after the caller has already converted (or, for a
 *  em-space-only weld pass, treated as the same numeric budget — em-unit
 *  coordinates are typically O(1e3), so this stays a no-op unless two
 *  points are truly coincident, exactly like the kernel's own use). */
const WELD_TOL = 1e-9

/**
 * Drops duplicate CONSECUTIVE points (within `tol`, default `WELD_TOL`) from
 * a contour — a closed loop's explicit closing point coinciding with its
 * start, or a flattened Bézier's endpoint landing on the next command's
 * start — including the closing edge (last → first), and drops a resulting
 * degenerate contour (fewer than 3 distinct points) entirely. Never reorders
 * points, and — unlike `weldContour` — never drops a contour on NET SIGNED
 * AREA alone: a self-crossing "bowtie" contour can have two lobes of
 * opposite winding that cancel to a near-zero net signed area while still
 * having real extent and a genuine self-crossing, so an area-based drop at
 * this layer would silently discard exactly the geometry
 * `outlineValidation.ts`'s self-intersection check exists to catch
 * (adversarial-review finding 3). `weldContour` adds that area drop on top
 * for callers (the real placement pipeline, `glyphRun.ts`) that want it;
 * this function is the shared dedupe step both build on.
 */
export function dedupeContourPoints(points: readonly Pt2[], tol: number = WELD_TOL): Pt2[] {
  if (points.length === 0) return []
  const out: Pt2[] = [points[0]]
  for (let i = 1; i < points.length; i++) {
    const prev = out[out.length - 1]
    const p = points[i]
    if (Math.hypot(p[0] - prev[0], p[1] - prev[1]) > tol) {
      out.push(p)
    }
  }
  // The closing edge (last → first) can also collapse to nothing.
  if (out.length > 1) {
    const first = out[0]
    const last = out[out.length - 1]
    if (Math.hypot(last[0] - first[0], last[1] - first[1]) <= tol) {
      out.pop()
    }
  }
  if (out.length < 3) return []
  return out
}

/**
 * `dedupeContourPoints` plus a degenerate-NET-AREA drop: a contour whose
 * shoelace signed area is near-zero is treated as no material at all. This
 * is the right call for the actual placement pipeline (`glyphRun.ts`) — a
 * zero-area loop has nothing to extrude — but is NOT safe for
 * `outlineValidation.ts`'s self-intersection warning, which must still see
 * a self-crossing contour even when its lobes cancel to zero net area; that
 * caller uses `dedupeContourPoints` directly instead (see its doc comment).
 */
export function weldContour(points: readonly Pt2[], tol: number = WELD_TOL): Pt2[] {
  const out = dedupeContourPoints(points, tol)
  if (out.length === 0) return []
  if (Math.abs(signedArea(out)) <= tol * tol) return []
  return out
}

/** Twice the signed area of a closed polygon (shoelace formula) — positive
 *  for CCW, negative for CW, in whatever winding the input happens to use
 *  (this module never assumes/normalizes a convention; only the SIGN
 *  relationships between contours matter for nesting, not an absolute
 *  orientation). */
export function signedArea(points: readonly Pt2[]): number {
  let sum = 0
  for (let i = 0; i < points.length; i++) {
    const [x1, y1] = points[i]
    const [x2, y2] = points[(i + 1) % points.length]
    sum += x1 * y2 - x2 * y1
  }
  return sum / 2
}

// ───────────────────────────────────────────────────────── fill classification

/**
 * True iff `p` is inside the closed polygon `poly` (standard even-odd
 * ray-casting test, boundary-exclusive). Pure point-in-polygon; no
 * winding-rule assumption about `poly`'s own orientation.
 */
export function pointInPolygon(p: Pt2, poly: readonly Pt2[]): boolean {
  let inside = false
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i]
    const [xj, yj] = poly[j]
    const intersects =
      yi > p[1] !== yj > p[1] && p[0] < ((xj - xi) * (p[1] - yi)) / (yj - yi) + xi
    if (intersects) inside = !inside
  }
  return inside
}

// ─────────────────────────────────────────────── nesting depth by interval

/** The plain vertex average — the last-resort probe for an input so
 *  degenerate (no vertical extent at all, or fewer than 3 points) that the
 *  scanline method below has no gap to work with. Such a contour is already
 *  numerically pathological (a horizontal sliver, or worse) and downstream
 *  sliver/degenerate filters handle the fallout, so a best-effort point
 *  here is the right failure mode, not a crash — this is never expected to
 *  run for real glyph geometry. Used only by `interiorPoint`, which must
 *  always hand back SOME point; `classifyContourFill`'s nesting-depth
 *  computation below has no equivalent probe to fall back on — a
 *  degenerate contour there is excluded outright (see
 *  `contourNestingDepth`'s doc comment). */
function centroidFallback(points: readonly Pt2[]): Pt2 {
  if (points.length === 0) return [0, 0]
  const inv = 1 / points.length
  return points.reduce<Pt2>((acc, p) => [acc[0] + p[0] * inv, acc[1] + p[1] * inv], [0, 0])
}

/** Every edge crossing of `poly` at horizontal line `y`, as x-coordinates,
 *  unsorted. Assumes `y` equals no vertex-y of `poly` (the caller always
 *  picks `y` from a gap between distinct sorted vertex-y values for exactly
 *  this reason), so every edge is either entirely on one side of `y` or
 *  straddles it with two DISTINCT endpoint y-values — never a vertex sitting
 *  exactly on the line, which would otherwise make an edge's crossing count
 *  ambiguous (counted once, twice, or zero times depending on its neighbor). */
function edgeCrossingsAtY(poly: readonly Pt2[], y: number): number[] {
  const xs: number[] = []
  for (let i = 0; i < poly.length; i++) {
    const [ax, ay] = poly[i]
    const [bx, by] = poly[(i + 1) % poly.length]
    if ((ay < y && by > y) || (ay > y && by < y)) {
      xs.push(ax + ((y - ay) / (by - ay)) * (bx - ax))
    }
  }
  return xs
}

/** `poly`'s OWN inside-intervals at horizontal line `y`: its own edge
 *  crossings — nobody else's — sorted and paired left-to-right by the
 *  even-odd rule (`[c0,c1]`, `[c2,c3]`, …), each pair the x-span where a
 *  ray at this `y` is inside `poly`. Computing a contour's interval from
 *  ONLY its own crossings (never merged with another contour's) is the
 *  crux of the fix this module exists for: two contours can share an edge
 *  and therefore tie a crossing at the same x, but that tie never
 *  contaminates either contour's OWN interval, because each is paired up
 *  independently before anyone compares them. Same non-ambiguity
 *  precondition as `edgeCrossingsAtY` (`y` equals no vertex-y of `poly`),
 *  so the crossing count is always even. */
function ownIntervalsAtY(poly: readonly Pt2[], y: number): ReadonlyArray<readonly [number, number]> {
  const xs = edgeCrossingsAtY(poly, y).sort((a, b) => a - b)
  const intervals: Array<[number, number]> = []
  for (let i = 0; i + 1 < xs.length; i += 2) intervals.push([xs[i], xs[i + 1]])
  return intervals
}

/** Midpoints of every gap between adjacent DISTINCT values in `ys` that
 *  falls entirely within `[minY, maxY]`, ordered narrowest gap first — the
 *  tightest bound is tried first, widening only if a narrower one turns
 *  out degenerate (see callers). */
function sortedGapMidpoints(ys: ReadonlySet<number>, minY: number, maxY: number): number[] {
  const sorted = Array.from(ys).sort((a, b) => a - b)
  const gaps: { lo: number; hi: number }[] = []
  for (let i = 0; i + 1 < sorted.length; i++) {
    const lo = sorted[i]
    const hi = sorted[i + 1]
    if (lo >= minY && hi <= maxY) gaps.push({ lo, hi })
  }
  gaps.sort((a, b) => a.hi - a.lo - (b.hi - b.lo))
  return gaps.map((g) => (g.lo + g.hi) / 2)
}

/**
 * Nesting depth of `contours[index]` (call it `X`) among every OTHER
 * contour in `contours`, computed by INTERVAL CONTAINMENT on a shared
 * scanline — no interior-point probe anywhere in this computation. This is
 * the structural replacement for the point-probe approach this module used
 * through three separate confirmed misclassifications (see the doc comment
 * on `kernel::Document::place_text` for the shape history): a probe placed
 * "just inside X" can always be fooled by SOME neighboring geometry closer
 * than expected, because a single point can only ever be tested against
 * *other contours' interiors*, and interiors of contours that share part of
 * X's own boundary have no clean way to be excluded from that test. An
 * interval has no such failure mode: `X`'s own inside-interval at a
 * scanline is computed from ONLY `X`'s own edges (`ownIntervalsAtY`), so it
 * is never distorted by a neighbor's geometry, tied crossing or not.
 *
 * Contours here are non-crossing kernel-split boundaries (either raw glyph
 * outlines or the kernel's own resolved region boundaries — never
 * self-intersecting, never partially overlapping), so another contour `C`
 * PROPERLY CONTAINS `X` iff `C`'s own interval at a shared scanline fully
 * covers `X`'s: `C.a <= X.enter && C.b >= X.exit`. Ties at the entry — two
 * contours sharing a vertical edge, routine for the kernel's own adjacent
 * post-split regions — resolve correctly for free, with no special-case
 * tie-breaking: a nested SIBLING's own interval necessarily ENDS BEFORE
 * `X`'s exit (it only covers part of `X`'s span, the reviewer's exact
 * confirmed failure: `N` touching `X`'s left edge but not spanning `X`'s
 * whole width), while a true CONTAINER's interval covers the whole span —
 * the two cases are geometrically distinguishable by the containment test
 * alone, because each interval was computed independently and never mixed
 * with the other's crossings in the first place.
 *
 * Returns `null` for a genuinely degenerate `X`: fewer than 3 points, no
 * vertical extent at all (a horizontal sliver — no scanline crosses it),
 * or every candidate scanline yields a zero-width own-interval (`X`'s
 * boundary folds back on itself, entering and exiting at the same x, at
 * every testable row). `classifyContourFill` treats `null` as "exclude,
 * never fill" — a sliver, not a guess.
 */
export function contourNestingDepth(
  contours: readonly (readonly Pt2[])[],
  index: number,
): number | null {
  const target = contours[index]
  if (target.length < 3) return null

  // Every distinct vertex-y across ALL contours (not just X): the scanline
  // must equal none of them, or ANY contour's own crossing count at that y
  // becomes ambiguous (a vertex sitting exactly on the line), the same
  // precondition `edgeCrossingsAtY` documents — needed here for every
  // contour tested, not only X.
  const allYs = new Set<number>()
  for (const c of contours) for (const [, y] of c) allYs.add(y)

  let targetMinY = Infinity
  let targetMaxY = -Infinity
  for (const [, y] of target) {
    if (y < targetMinY) targetMinY = y
    if (y > targetMaxY) targetMaxY = y
  }
  if (!(targetMinY < targetMaxY)) return null // a horizontal sliver: no scanline crosses it

  for (const y of sortedGapMidpoints(allYs, targetMinY, targetMaxY)) {
    const ownIntervals = ownIntervalsAtY(target, y)
    if (ownIntervals.length === 0) continue // defensive; see edgeCrossingsAtY's precondition

    // X may have SEVERAL own intervals at this y (a concave/multi-lobe
    // contour — an 'M' or 'W' stroke, say); use the leftmost, the same
    // interval the prior first-entering-crossing probe always anchored on.
    const [enterX, exitX] = ownIntervals[0]
    if (!(exitX > enterX)) continue // degenerate at this y — try the next gap

    let depth = 0
    for (let j = 0; j < contours.length; j++) {
      if (j === index) continue
      const other = contours[j]
      if (other.length < 3) continue
      for (const [a, b] of ownIntervalsAtY(other, y)) {
        if (a === enterX && b === exitX) {
          // Exact-equality interval: contours are non-crossing, so when
          // another contour's interval at this scanline exactly equals
          // X's, their interiors coincide at this row — one must contain
          // the other (this is the reviewer's confirmed repro: a
          // full-width child spanning its container's entire width at
          // the sampled row shares BOTH walls with it, tying `a`/`b`
          // exactly, yet is genuinely nested). Resolve by vertical
          // extent instead of the shared row: C contains X iff C's total
          // y-extent PROPERLY covers X's (min-y <= and max-y >=, with at
          // least one strict) — a true container's boundary must reach
          // further up or down than a same-width nested child to enclose
          // it, even though the two are indistinguishable at this one
          // shared row. Exactly equal y-extents too would mean
          // coincident contours, which the kernel never emits — treat
          // as not-containing. (The mirrored case — X's extent properly
          // covering C's — means X contains C, so C does not contain X:
          // also not-containing.)
          let otherMinY = Infinity
          let otherMaxY = -Infinity
          for (const [, oy] of other) {
            if (oy < otherMinY) otherMinY = oy
            if (oy > otherMaxY) otherMaxY = oy
          }
          if (
            otherMinY <= targetMinY &&
            otherMaxY >= targetMaxY &&
            (otherMinY < targetMinY || otherMaxY > targetMaxY)
          ) {
            depth++
            break
          }
          continue
        }
        if (a <= enterX && b >= exitX) {
          depth++
          break // C is simple/non-crossing: at most one of its own
          // intervals can contain X's at this scanline, so no other
          // interval of C needs checking once one match is found.
        }
      }
    }
    return depth
  }
  return null // every candidate scanline degenerated: a sliver, not real material
}

/**
 * A point GUARANTEED to lie strictly inside the closed simple polygon
 * `points` (never on its boundary) — `points`' own leftmost inside-interval
 * at a scanline chosen to avoid every one of `points`' own vertex-y values
 * (`sortedGapMidpoints`/`ownIntervalsAtY`, narrowest gap tried first,
 * widening only on a degenerate row), landed at that interval's midpoint.
 *
 * This is deliberately simpler than the nesting-depth machinery above: with
 * no sibling contours in the picture at all, there is no tie to dodge and
 * no neighbor whose geometry could distort the result — `points`' own
 * crossings are the only input, so the interval they pair into is always
 * `points`' true cross-section, never contaminated by anyone else's edges.
 * See the call site in `TextPlaceTool.ts` for why landing this probe from
 * `points` ALONE (not measured against sibling contours) is safe for its
 * one caller (glyph-ink attribution, not sibling nesting).
 *
 * Falls back to the plain vertex average (`centroidFallback`) for a
 * degenerate input (fewer than 3 points, no vertical extent at all, or
 * every candidate scanline degenerate) rather than throwing.
 */
export function interiorPoint(points: readonly Pt2[]): Pt2 {
  if (points.length < 3) return centroidFallback(points)

  const ys = new Set<number>()
  let minY = Infinity
  let maxY = -Infinity
  for (const [, y] of points) {
    ys.add(y)
    if (y < minY) minY = y
    if (y > maxY) maxY = y
  }
  if (!(minY < maxY)) return centroidFallback(points) // a horizontal sliver

  for (const y of sortedGapMidpoints(ys, minY, maxY)) {
    const intervals = ownIntervalsAtY(points, y)
    if (intervals.length === 0) continue
    const [a, b] = intervals[0] // leftmost interval, same choice as contourNestingDepth
    if (b > a) return [(a + b) / 2, y]
  }
  return centroidFallback(points) // every candidate scanline degenerated
}

/**
 * Classifies every contour in `contours` as glyph FILL (material, `true`)
 * or a counter's own bare interior (`false`, never extrude) — the
 * even-odd/nonzero nesting-depth rule every font rasterizer uses, computed
 * purely geometrically so it works regardless of a font's internal
 * winding convention (TrueType and CFF wind outer-vs-counter oppositely):
 * for each contour, `contourNestingDepth` counts how many OTHER contours
 * properly CONTAIN it by INTERVAL CONTAINMENT on a shared scanline — no
 * interior-point probe (see that function's doc comment for why the
 * probe-based approach this replaces kept failing: a point can always be
 * fooled by some neighboring geometry the point happens to land inside,
 * however that neighbor relates to the target contour; an interval,
 * computed from ONLY the target's own edges, cannot be). An EVEN count (0,
 * 2, 4, …) is fill; an ODD count (1, 3, …) is a hole. A degenerate contour
 * (`contourNestingDepth` returns `null` — too few points, no vertical
 * extent, or every scanline yields a zero-width own-interval) is excluded
 * outright (`false`, never extrude) rather than guessed at: a genuine
 * sliver is not glyph material either way, and there is no probe left here
 * to fall back on. An 'O' — two nested loops — classifies as
 * `[true, false]` (or `[false, true]`, order depends on the input): the
 * outer contour (contained by nothing) is fill, the counter contour
 * (contained by the outer) is a hole.
 */
export function classifyContourFill(contours: readonly Pt2[][]): boolean[] {
  return contours.map((_, i) => {
    const depth = contourNestingDepth(contours, i)
    if (depth === null) return false
    return depth % 2 === 0
  })
}
