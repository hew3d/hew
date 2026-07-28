/**
 * TextPlaceTool — 3D Text placement (docs/design/3d-text.md).
 *
 * Armed by the "3D Text…" dialog with the resolved glyph geometry (already
 * flattened + classified fill/hole by `text/glyphRun.ts`) and a target
 * depth/name; the ghost then follows the cursor on whatever plane the
 * draw-tool rules resolve (hovered face, ground, arrow-key axis lock — the
 * same rules RectangleTool/CircleTool use, minus their sketch-mode
 * adoption branch, which 3D Text has no use for: its own sketch is always
 * freshly created and fully consumed by the placement, never shared).
 *
 * A single click commits: injects every contour's edges as ONE
 * gesture-bracketed sketch (`begin_sketch_on_plane` → `sketch_begin_gesture`
 * → `sketch_add_segment` per edge → `sketch_end_gesture`), resolves the
 * kernel's closed regions (`sketch_regions`), classifies exactly which of
 * THOSE resolved regions are fill by the same even-odd nesting-depth rule
 * `classifyContourFill` applies to raw contours — but applied to the
 * regions' OWN outer boundaries (`region_boundary`) AFTER the kernel has
 * already split/closed them, not to the original pre-injection glyph
 * contours. This matters: some fonts' glyphs (observed while screening
 * candidate bundled families — a variable font's default instance is the
 * raw `glyf` outline with no `gvar` deltas applied, and that raw outline
 * is not always well-formed; see `text/fonts.ts`'s bundled-font doc
 * comment) encode a counter as a single self-touching ("pinched") contour
 * rather than two separate loops, so classifying the ORIGINAL contours
 * independently cannot tell fill from hole for them — the kernel's region
 * resolver splits the pinch into the correct two regions regardless, and
 * classifying THOSE (not the pre-split input) is what stays correct for
 * every glyph shape, bundled or user-loaded. Then calls
 * `place_text` — the kernel's own atomic tail that extrudes the selected
 * regions, folds the results into one component, and records the whole
 * placement (gesture included) as ONE undo step.
 *
 * The tool stays armed after a successful placement (this codebase's
 * existing convention for action tools — SliceTool, Follow Me — which the
 * user exits explicitly by switching tools), so "Hew 3D!" can be stamped
 * more than once without reopening the dialog.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { rayPlaneIntersect, facePlaneBasis } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'
import {
  GLYPH_SAGITTA_TOL_M,
  classifyContourFill,
  interiorPoint,
  pointInPolygon,
  signedArea,
  type Pt2,
} from '../text/flatten'
import type { GlyphRunContour } from '../text/glyphRun'
import { axisDrawPlane, groundDrawPlane, type DrawPlane } from './drawPlane'
import { defaultFaceEligible, FacePickCache, type FaceEligible } from './faceDraw'
import { nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'

export type OnPlaced = (instanceId: bigint) => void
export type OnToast = (message: string, code?: string) => void

/** The resolved glyph geometry + placement parameters the dialog hands the
 *  tool — everything font/text-specific; the tool itself only ever sees
 *  plain contours and numbers. */
export interface TextPlacement {
  /** Flattened, welded, fill-classified contours in the font's own
   *  em-square units (see `text/glyphRun.ts`). */
  contours: GlyphRunContour[]
  /** World-units-per-em-unit scale (`heightMeters / font.unitsPerEm`). */
  scale: number
  /** Extrusion depth in world units (meters); must be positive. */
  depth: number
  /** The new component definition's name, e.g. `3D Text "Hew"`. */
  name: string
}

/** Below this fraction of the largest candidate region's area WITHIN THE
 *  SAME GLYPH (`glyphIndexOf`, see `selectFillRegions`), a region is
 *  treated as a numerical sliver, not a real glyph feature — dropped
 *  before fill/hole classification even runs (see `selectFillRegions`'s
 *  doc comment for why real font outlines need this). Scoped per glyph
 *  rather than across the whole run: real text lays every glyph out at
 *  its own non-overlapping advance-width slot, so a legitimate small mark
 *  (a period) sitting next to a much larger glyph (a notdef box) is not a
 *  sliver of THAT glyph — judging it against the run's overall largest
 *  region would drop it silently. */
const SLIVER_AREA_RATIO = 0.01

/** Absolute area floor (world m²), independent of any glyph's own relative
 *  scale — catches a genuinely degenerate sliver even for a glyph whose
 *  ONLY candidate region IS the sliver itself, where a per-glyph ratio
 *  (always exactly 1.0 against its own single candidate) could never
 *  filter it. Derived from `GLYPH_SAGITTA_TOL_M` — the same sub-print /
 *  below-silhouette-visibility tolerance this feature's own curve
 *  flattening already commits to (`flatten.ts`): a region narrower than
 *  that tolerance on a side is already below what the tessellation
 *  itself resolves faithfully, so treating it as "not a real feature" is
 *  consistent with the tolerance the rest of this pipeline uses, not an
 *  arbitrary extra number. */
export const MIN_FEATURE_AREA_M2 = GLYPH_SAGITTA_TOL_M * GLYPH_SAGITTA_TOL_M

/** Picks exactly the FILL regions among a sketch's resolved closed regions
 *  — the same even-odd nesting-depth rule `classifyContourFill` applies to
 *  raw glyph contours, but applied HERE to the regions' own outer
 *  boundaries, after the kernel has already resolved/split them. See the
 *  module doc for why this must run post-resolution rather than on the
 *  pre-injection contours (a pinched/self-touching glyph contour can't be
 *  classified correctly before the kernel's own splitting has happened).
 *  `outerBoundaries` are each region's outer-loop points in the SAME 2D
 *  frame (any consistent in-plane coordinates; only relative containment
 *  matters, not units).
 *
 * Real font outlines — even from well-established OFL families — are not
 * always simple polygons at the RAW outline level: several tested fonts'
 * variable-font default instance (the master `glyf` stores without
 * applying any `gvar` deltas, which this app never resolves) render some
 * letters with genuinely self-crossing contours (observed while screening
 * candidates for `text/fonts.ts`'s bundled set: Inter's 'e' and 'w',
 * Manrope's 'a', Work Sans's 'e'/'a', Fredoka's 'M'/'6'/'9' — a widespread
 * trait of current variable-only Google Fonts releases, not one bad font
 * choice, which is why this guard exists even though the shipped bundled
 * fonts were screened clean; a user-loaded font gets no such screening).
 * The kernel's sticky topology still resolves a definite, valid set of
 * regions from that — it just includes a few vanishingly small sliver
 * regions at each self-crossing, alongside the real letterform. Slivers
 * are dropped by AREA before classification (below `SLIVER_AREA_RATIO` of
 * the largest SAME-GLYPH candidate's area, OR below `absoluteAreaFloor`)
 * rather than trying to repair the input contour itself — repair would
 * risk silently reshaping a shape the kernel can already resolve exactly;
 * dropping a numerically-negligible artifact cell is not a claim about
 * the letter's real geometry.
 *
 * `glyphIndexOf[i]` is which glyph `outerBoundaries[i]` belongs to
 * (`GlyphRunContour.glyphIndex`, attributed post-resolution by the
 * caller); omitted, every region is treated as one group — today's
 * whole-run behavior, which is also exactly right for a single-glyph
 * caller (this function's own unit tests below). `absoluteAreaFloor`
 * defaults to `0` (no absolute floor) for the same reason: it must be
 * expressed in `outerBoundaries`' OWN units, which this function does not
 * otherwise assume anything about.
 *
 * `ambiguousAt[i]` marks a region whose `glyphIndexOf[i]` attribution came
 * from `glyphIndexAt`'s AMBIGUOUS-overlap tie-break (its interior probe
 * sat in more than one glyph's own ink, see that function's doc comment)
 * rather than a clean single-candidate match. Such a region is excluded
 * from `groupMax` — the whole POINT of the per-glyph floor is normalizing
 * a group's sliver threshold to THAT glyph's own scale, and an ambiguously
 * -attributed region may really belong to an entirely different (often
 * much larger) glyph, so folding its area into this group's max would
 * scale the floor to the WRONG glyph's size, not this one's — the exact
 * failure this flag exists to prevent: a large region misattributed into
 * a small glyph's group inflating that glyph's own floor high enough to
 * drop the small glyph's genuine small ink as a "sliver". It is also
 * NEVER dropped by the floor itself (unconditionally kept through to
 * fill/hole classification below): an ambiguous region is, by
 * construction, real overlap ink — it sat inside actual glyph outline
 * material for more than one glyph — never a numerical artifact, so
 * there is no floor it could legitimately fail.
 */
export function selectFillRegions<R>(
  regionIds: readonly R[],
  outerBoundaries: readonly Pt2[][],
  glyphIndexOf?: readonly number[],
  absoluteAreaFloor = 0,
  ambiguousAt?: readonly boolean[],
): R[] {
  const areas = outerBoundaries.map((b) => Math.abs(signedArea(b)))
  const groupOf = (i: number): number => glyphIndexOf?.[i] ?? 0
  const isAmbiguous = (i: number): boolean => ambiguousAt?.[i] ?? false
  const groupMax = new Map<number, number>()
  areas.forEach((a, i) => {
    if (isAmbiguous(i)) return // never scales another glyph's own floor
    const g = groupOf(i)
    groupMax.set(g, Math.max(groupMax.get(g) ?? 0, a))
  })
  const keptIdx: number[] = []
  const keptBoundaries: Pt2[][] = []
  areas.forEach((a, i) => {
    if (isAmbiguous(i)) {
      // Real overlap ink, by construction — bypasses the sliver floor
      // entirely rather than being judged against a group it may not
      // even truly belong to.
      keptIdx.push(i)
      keptBoundaries.push(outerBoundaries[i])
      return
    }
    const floor = Math.max((groupMax.get(groupOf(i)) ?? 0) * SLIVER_AREA_RATIO, absoluteAreaFloor)
    if (a >= floor) {
      keptIdx.push(i)
      keptBoundaries.push(outerBoundaries[i])
    }
  })
  const fills = classifyContourFill(keptBoundaries)
  return keptIdx.filter((_, k) => fills[k]).map((i) => regionIds[i])
}

/** Per-glyph em-space bounding box (min/max X and Y across every one of
 *  that glyph's own pre-injection contour points). No longer used to
 *  ATTRIBUTE a point to a glyph (`glyphIndexAt` below tests real outline
 *  containment for that) — kept for the ambiguous-overlap tie-break
 *  (`glyphIndexAt`'s ink-space candidates are scored by their glyph's box
 *  area) and as the final belt-and-suspenders fallback when no glyph's ink
 *  actually contains the point at all. */
export function glyphBoundingBoxes(
  contours: readonly GlyphRunContour[],
): Map<number, { minX: number; maxX: number; minY: number; maxY: number }> {
  const boxes = new Map<number, { minX: number; maxX: number; minY: number; maxY: number }>()
  for (const c of contours) {
    let box = boxes.get(c.glyphIndex)
    for (const [x, y] of c.points) {
      if (box === undefined) {
        box = { minX: x, maxX: x, minY: y, maxY: y }
        boxes.set(c.glyphIndex, box)
      } else {
        if (x < box.minX) box.minX = x
        if (x > box.maxX) box.maxX = x
        if (y < box.minY) box.minY = y
        if (y > box.maxY) box.maxY = y
      }
    }
  }
  return boxes
}

/** Every one of `contours`' own points (both fill AND hole contours),
 *  grouped by `glyphIndex` — the raw material `glyphIndexAt` even-odd-tests
 *  a candidate point against, one glyph at a time. Holes are included
 *  deliberately: a point sitting in a glyph's own counter (its hole, e.g.
 *  inside an 'O') must NOT read as that glyph's ink, and even-odd
 *  containment across the WHOLE contour set (fill and hole alike) is
 *  exactly what makes that fall out correctly — the same rule
 *  `classifyContourFill` itself applies, just scoped to one glyph's own
 *  contours instead of a whole run/region set. */
export function glyphContoursByIndex(
  contours: readonly GlyphRunContour[],
): Map<number, Pt2[][]> {
  const byGlyph = new Map<number, Pt2[][]>()
  for (const c of contours) {
    const list = byGlyph.get(c.glyphIndex)
    if (list !== undefined) list.push(c.points)
    else byGlyph.set(c.glyphIndex, [c.points])
  }
  return byGlyph
}

/** True iff `point` lies in `glyphContours`' own ink (an odd number of that
 *  glyph's OWN contours contain it) — the even-odd fill rule applied to one
 *  glyph's contour set alone, so a point inside that glyph's hole (an even
 *  depth: inside the outer AND inside the counter) correctly reads as NOT
 *  ink, while a point inside a deeper nested fill (rare, but the general
 *  rule handles it) still reads as ink. */
function pointInGlyphInk(point: Pt2, glyphContours: readonly Pt2[][]): boolean {
  let depth = 0
  for (const c of glyphContours) {
    if (pointInPolygon(point, c)) depth++
  }
  return depth % 2 === 1
}

function glyphBoxArea(
  box: { minX: number; maxX: number; minY: number; maxY: number } | undefined,
): number {
  if (box === undefined) return Infinity
  return (box.maxX - box.minX) * (box.maxY - box.minY)
}

/**
 * Which glyph (by index) `point` (em-space) belongs to, and whether that
 * attribution was AMBIGUOUS — by testing `point` against each glyph's
 * actual OUTLINE CONTOURS (even-odd, via `pointInGlyphInk`), not its
 * bounding EM-BOX. A bounding-box test misattributes under overlapping
 * em-boxes: tight or negative kerning, an italic overhang, or an
 * "AV"/"To"-style pair can make two glyphs' boxes overlap even though
 * their actual ink never does — a resolved region physically built from
 * one glyph's edges could then get box-tested against the WRONG glyph
 * first and silently borrow that glyph's sliver floor instead of its own.
 *
 * `point` should be the region's own interior PROBE (`interiorPoint` from
 * `flatten.ts`), not a boundary vertex — the same reasoning
 * `classifyContourFill` documents: a boundary point can sit exactly on a
 * neighboring glyph's own edge, tying the even-odd test ambiguously.
 *
 * Real placed-text glyphs never actually share ink (each glyph flattens
 * from its own advance-width slot with no shared geometry), so in the
 * ordinary case at most one glyph's ink contains `point` and this returns
 * it directly, `ambiguous: false`. Genuinely AMBIGUOUS overlap (more than
 * one glyph's ink contains the same point — reachable only through a
 * pathological/self-touching font outline, since the design never
 * intentionally overlaps glyphs) resolves `index` to the SMALLER
 * candidate glyph by em-box area (conservative tie-break: a smaller
 * glyph's own scale is smaller, so if `index` were ever used to scope a
 * per-glyph floor directly, erring toward the smaller glyph keeps detail
 * rather than dropping it) — but ALSO reports `ambiguous: true`, because
 * `index` here is still just a resolved GUESS between two candidates, not
 * a confirmed single match. Callers that feed a per-glyph scale floor
 * (`selectFillRegions`'s `groupMax`, see its doc comment) must treat an
 * ambiguous region as untrustworthy for THAT purpose — the region may
 * really belong to the OTHER candidate, often a much larger glyph — even
 * though `index` still names a glyph for any purpose that merely needs
 * *a* label (logging, preview grouping) rather than a scale-normalization
 * input.
 *
 * Falls back to the nearest box's center — the same belt-and-suspenders
 * tie-break the old bounding-box version used — only when NO glyph's ink
 * actually contains the point at all (a numerical edge case exactly on a
 * shared boundary); `ambiguous: false` in this case too, since there is
 * only ever one nearest box, not competing candidates. `index` is `-1` if
 * there are no boxes at all (nothing placed yet).
 */
export function glyphAttribution(
  point: Pt2,
  contoursByGlyph: ReadonlyMap<number, Pt2[][]>,
  boxes: ReadonlyMap<number, { minX: number; maxX: number; minY: number; maxY: number }>,
): { index: number; ambiguous: boolean } {
  const candidates: number[] = []
  for (const [idx, contours] of contoursByGlyph) {
    if (pointInGlyphInk(point, contours)) candidates.push(idx)
  }
  if (candidates.length === 1) return { index: candidates[0], ambiguous: false }
  if (candidates.length > 1) {
    let best = candidates[0]
    let bestArea = glyphBoxArea(boxes.get(best))
    for (const idx of candidates.slice(1)) {
      const area = glyphBoxArea(boxes.get(idx))
      if (area < bestArea) {
        best = idx
        bestArea = area
      }
    }
    return { index: best, ambiguous: true }
  }
  let best = -1
  let bestDist = Infinity
  for (const [idx, b] of boxes) {
    const cx = (b.minX + b.maxX) / 2
    const cy = (b.minY + b.maxY) / 2
    const d = Math.hypot(point[0] - cx, point[1] - cy)
    if (d < bestDist) {
      bestDist = d
      best = idx
    }
  }
  return { index: best, ambiguous: false }
}

/** `glyphAttribution(...).index` alone — for callers (and existing tests)
 *  that only need the resolved glyph label, not the ambiguity flag; see
 *  `glyphAttribution`'s doc comment for the full resolution rules and,
 *  critically, why a caller feeding a per-glyph SCALE floor must use
 *  `glyphAttribution` directly instead of dropping the flag here. */
export function glyphIndexAt(
  point: Pt2,
  contoursByGlyph: ReadonlyMap<number, Pt2[][]>,
  boxes: ReadonlyMap<number, { minX: number; maxX: number; minY: number; maxY: number }>,
): number {
  return glyphAttribution(point, contoursByGlyph, boxes).index
}

/** Assigns each hole-classified contour to the smallest fill contour whose
 *  polygon contains one of its points — purely for the GHOST PREVIEW's
 *  visual fidelity (the kernel does the authoritative region/hole
 *  resolution on commit; this never feeds back into placement). Contours
 *  with no enclosing fill shape are dropped from the preview. */
function assignHolesToFills(
  contours: readonly GlyphRunContour[],
): Array<{ outer: Pt2[]; holes: Pt2[][] }> {
  const fills = contours.filter((c) => c.fill)
  const holes = contours.filter((c) => !c.fill)
  const shapes = fills.map((f) => ({ outer: f.points, holes: [] as Pt2[][] }))
  for (const hole of holes) {
    const probe = hole.points[0]
    let bestIdx = -1
    let bestArea = Infinity
    fills.forEach((f, i) => {
      if (!pointInPolygon(probe, f.points)) return
      const area = Math.abs(polygonArea(f.points))
      if (area < bestArea) {
        bestArea = area
        bestIdx = i
      }
    })
    if (bestIdx >= 0) shapes[bestIdx].holes.push(hole.points)
  }
  return shapes
}

function polygonArea(points: readonly Pt2[]): number {
  let sum = 0
  for (let i = 0; i < points.length; i++) {
    const [x1, y1] = points[i]
    const [x2, y2] = points[(i + 1) % points.length]
    sum += x1 * y2 - x2 * y1
  }
  return sum / 2
}

/** Idle stage: no click yet, ghost follows the resolved plane. */
type Stage = { kind: 'idle' }

export class TextPlaceTool implements Tool {
  readonly name = 'Text'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private placement: TextPlacement
  private onPlaced: OnPlaced
  private onToast: OnToast

  private _activeGroup: bigint | null = null
  private _componentContext: bigint | null = null
  private _activeContext: bigint | null = null
  private _faceEligible: FaceEligible | null = null
  private readonly _pickCache = new FacePickCache()

  /** Idle arrow-key plane lock (mirrors RectangleTool's design §5.2 idle
   *  lock — the same convention every draw tool shares). */
  private idlePlaneLock: 0 | 1 | 2 | null = null
  private _lastHoverPlane: DrawPlane | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    placement: TextPlacement,
    onPlaced: OnPlaced,
    onToast: OnToast,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.placement = placement
    this.onPlaced = onPlaced
    this.onToast = onToast
  }

  /** Group-context placement (Follow Me precedent): the instance born
   *  while editing this group lands inside it; `null` births top-level —
   *  also the fallback while editing a component DEFINITION (no
   *  component-nesting surface exists yet), matched by `statusHint`. */
  setActiveGroup(id: bigint | null): void {
    this._activeGroup = id
  }

  /** Display-only: the component definition being edited, if any (drives
   *  `statusHint`'s note that placement still lands at the top level). */
  setComponentContext(id: bigint | null): void {
    this._componentContext = id
  }

  setActiveContext(id: bigint | null): void {
    this._activeContext = id
  }

  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  private _isEligible(objectHandle: bigint, instanceHandle: bigint | undefined): boolean {
    if (this._faceEligible !== null) return this._faceEligible(objectHandle, instanceHandle)
    return defaultFaceEligible(this.wasmScene, this._activeContext, objectHandle, instanceHandle)
  }

  statusHint(): string {
    if (this._componentContext !== null) {
      return 'Click to place — 3D Text places at the top level while editing a component.'
    }
    if (this.idlePlaneLock !== null) {
      return `Locked to the ${AXIS_LOCK_COLOR_NAMES[this.idlePlaneLock]} plane — click to place; same arrow or Esc unlocks.`
    }
    return 'Click a face, the ground, or an arrow-locked plane to place the text.'
  }

  /** Resolve the plane THIS pointer event places onto: an active idle axis
   *  lock beats face pick and the ground plane (every draw tool's rule —
   *  RectangleTool.ts's `_currentMode`, design §5.2: "an active idle plane
   *  lock beats face pick and sketch-hover adoption... the user already
   *  chose a plane"); otherwise an eligible face wins; otherwise the ground
   *  plane. No sketch-mode adoption (design doc's plane rules for 3D Text
   *  list only face/ground/axis-lock — this tool's sketch is always
   *  freshly created, never shared with an existing one). */
  private _resolvePlane(ray: Ray): { plane: DrawPlane; origin: V3 } | null {
    if (this.idlePlaneLock !== null) {
      const plane = axisDrawPlane(this.idlePlaneLock, [0, 0, 0])
      const hit = rayPlaneIntersect(ray.origin, ray.direction, plane.origin, plane.normal)
      if (hit === null) return null
      // The axis-lock plane must pass through the actual hover point, not a
      // fixed origin — rebuild it through `hit` (mirrors RectangleTool's
      // idle lock cue, `axisDrawPlane(lock, hoverPoint)`).
      return { plane: axisDrawPlane(this.idlePlaneLock, hit), origin: hit }
    }
    const eligible = this._pickCache.pickFor(this.wasmScene, ray, (o, i) => this._isEligible(o, i))
    if (eligible !== null) {
      const a = this.wasmScene.face_plane(eligible.object, eligible.face)
      const point: V3 = [a[0], a[1], a[2]]
      const normal: V3 = [a[3], a[4], a[5]]
      const basis = facePlaneBasis(normal)
      if (basis === null) return null
      const hit = rayPlaneIntersect(ray.origin, ray.direction, point, normal)
      if (hit === null) return null
      return { plane: { origin: point, normal, u: basis.u, v: basis.v, ground: false }, origin: hit }
    }
    const plane = groundDrawPlane()
    const hit = rayPlaneIntersect(ray.origin, ray.direction, plane.origin, plane.normal)
    if (hit === null) return null
    return { plane, origin: hit }
  }

  /** Map every contour's em-space points onto `plane` at `origin` — em X
   *  runs along `plane.u` (reading direction), em Y (canvas-down) runs
   *  along the NEGATIVE of `plane.v` so the text reads upright with `v` as
   *  "up". */
  private _worldContours(plane: DrawPlane, origin: V3): { points: V3[]; fill: boolean }[] {
    const { scale } = this.placement
    const map = (p: Pt2): V3 => {
      const eu = p[0] * scale
      const ev = -p[1] * scale
      return [
        origin[0] + plane.u[0] * eu + plane.v[0] * ev,
        origin[1] + plane.u[1] * eu + plane.v[1] * ev,
        origin[2] + plane.u[2] * eu + plane.v[2] * ev,
      ]
    }
    return this.placement.contours.map((c) => ({ points: c.points.map(map), fill: c.fill }))
  }

  onPointerMove(_snap: Snap | null, ray: Ray): void {
    const resolved = this._resolvePlane(ray)
    clearPreview(this.preview)
    if (resolved === null) return
    const { plane, origin } = resolved
    this._lastHoverPlane = plane

    // Ghost fill/hole grouping (visual only — the kernel resolves the
    // authoritative regions on commit); mapped through the exact same
    // em→world transform `_worldContours`/`_commit` use for the real
    // injected geometry, so the ghost matches what will actually be built.
    const emShapes = assignHolesToFills(this.placement.contours)
    const { scale } = this.placement
    const mapPt = (p: Pt2): V3 => {
      const eu = p[0] * scale
      const ev = -p[1] * scale
      return [
        origin[0] + plane.u[0] * eu + plane.v[0] * ev,
        origin[1] + plane.u[1] * eu + plane.v[1] * ev,
        origin[2] + plane.u[2] * eu + plane.v[2] * ev,
      ]
    }

    const group = new THREE.Group()
    const material = new THREE.MeshBasicMaterial({
      color: 0x3a7ce0,
      transparent: true,
      opacity: 0.45,
      depthWrite: false,
      side: THREE.DoubleSide,
    })
    for (const shape of emShapes) {
      const threeShape = new THREE.Shape(shape.outer.map(([x, y]) => new THREE.Vector2(x, y)))
      for (const hole of shape.holes) {
        threeShape.holes.push(new THREE.Path(hole.map(([x, y]) => new THREE.Vector2(x, y))))
      }
      const extrudeDepthEm = this.placement.depth / scale
      const geom = new THREE.ExtrudeGeometry(threeShape, {
        depth: extrudeDepthEm,
        bevelEnabled: false,
        curveSegments: 1,
      })
      // The shape lives in the (em-x, em-y) plane with extrusion along its
      // own +Z; reposition each vertex through the SAME world mapping (u,
      // v, normal) used for the real injected geometry.
      const pos = geom.attributes.position
      const scratch = new THREE.Vector3()
      for (let i = 0; i < pos.count; i++) {
        scratch.fromBufferAttribute(pos, i)
        const world = mapPt([scratch.x, scratch.y])
        const along = scratch.z * scale
        pos.setXYZ(
          i,
          world[0] + plane.normal[0] * along,
          world[1] + plane.normal[1] * along,
          world[2] + plane.normal[2] * along,
        )
      }
      pos.needsUpdate = true
      geom.computeVertexNormals()
      group.add(new THREE.Mesh(geom, material))
    }
    this.preview.add(group)
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    const resolved = this._resolvePlane(ray)
    if (resolved === null) return
    const { plane, origin } = resolved
    this._commit(plane, origin)
  }

  private _commit(plane: DrawPlane, origin: V3): void {
    const worldContours = this._worldContours(plane, origin)
    if (worldContours.length === 0) return

    let sketch: bigint
    try {
      sketch = this.wasmScene.begin_sketch_on_plane(
        origin[0], origin[1], origin[2],
        plane.normal[0], plane.normal[1], plane.normal[2],
      )
    } catch (err) {
      this._toastError(err)
      return
    }

    try {
      this.wasmScene.sketch_begin_gesture(sketch)
      for (const contour of worldContours) {
        const pts = contour.points
        for (let i = 0; i < pts.length; i++) {
          const a = pts[i]
          const b = pts[(i + 1) % pts.length]
          this.wasmScene.sketch_add_segment(sketch, a[0], a[1], a[2], b[0], b[1], b[2])
        }
      }
    } catch (err) {
      // Nothing committed yet (the gesture is still open, unclosed) —
      // close it so the tool doesn't leave a dangling pending gesture,
      // matching `runSketchGesture`'s finally-close contract; an
      // unchanged/partial gesture records nothing on cancel-equivalent
      // close paths, but `sketch_end_gesture` still records whatever DID
      // get added, so this stays a genuine (if minimal) undo step rather
      // than silently losing it.
      this.wasmScene.sketch_end_gesture(sketch)
      this._toastError(err)
      return
    }
    this.wasmScene.sketch_end_gesture(sketch)

    // Resolve which of the kernel's closed regions are glyph MATERIAL (see
    // the module doc + `kernel::Document::place_text`'s doc comment):
    // classify each resolved region's OWN outer boundary by nesting depth
    // among the OTHER resolved regions — post-split, so a pinched glyph
    // contour classifies correctly even though it was never two separate
    // input loops (see `selectFillRegions`'s doc comment).
    const toPlaneUV = (world: V3): Pt2 => {
      const dx = world[0] - origin[0]
      const dy = world[1] - origin[1]
      const dz = world[2] - origin[2]
      return [
        dx * plane.u[0] + dy * plane.u[1] + dz * plane.u[2],
        dx * plane.v[0] + dy * plane.v[1] + dz * plane.v[2],
      ]
    }
    const regionIds = Array.from(this.wasmScene.sketch_regions(sketch))
    const outerBoundaries: Pt2[][] = regionIds.map((region) => {
      let boundary: Float32Array
      try {
        boundary = this.wasmScene.region_boundary(sketch, region)
      } catch {
        return []
      }
      const pts: Pt2[] = []
      for (let i = 0; i + 2 < boundary.length; i += 3) {
        pts.push(toPlaneUV([boundary[i], boundary[i + 1], boundary[i + 2]]))
      }
      return pts
    })
    // Attribute each resolved region back to "which glyph" (for
    // `selectFillRegions`'s PER-GLYPH sliver floor — see its doc comment):
    // `outerBoundaries` are world-plane-UV (meters); undo the SAME
    // em→world scale `_worldContours` applied to recover em-space, the
    // frame `placement.contours`' own glyph outlines are in. Attributed by
    // testing each region's own interior PROBE (`interiorPoint` — never a
    // boundary vertex, see `glyphAttribution`'s doc comment) against every
    // glyph's actual outline ink (`glyphAttribution`), not a bounding box —
    // overlapping em-boxes (tight/negative kerning, an italic overhang)
    // would otherwise misattribute a region under overlapping boxes even
    // though real glyph ink never actually overlaps for placed text.
    // `glyphAttribution` (not the plain `glyphIndexAt` label) so the
    // AMBIGUOUS-overlap flag reaches `selectFillRegions`: an ambiguously
    // -attributed region's area must never scale a glyph group's sliver
    // floor (see `selectFillRegions`'s `ambiguousAt` doc comment) — it may
    // really belong to an entirely different, often much larger, glyph.
    //
    // `interiorPoint` here is `b`'s OWN interval midpoint (its leftmost
    // inside-interval at a scanline that avoids only `b`'s own vertex-y
    // values) — it never measures `b` against any sibling contour. That is
    // safe for THIS call specifically: nesting among resolved regions was
    // already fully decided by `contourNestingDepth`/`classifyContourFill`
    // (the interval-containment replacement for the point-probe approach
    // this module used to share) before `selectFillRegions` ever runs, so
    // nothing downstream of this point needs `probe` to resolve nesting.
    // Its only remaining job is picking a point provably inside `b` to test
    // against each GLYPH's own ink (`glyphAttribution`) — and real placed
    // glyphs never share ink with one another (each flattens from its own
    // non-overlapping advance-width slot), so there is no sibling contour
    // here for a probe to be misled by in the first place, unlike the
    // sibling-nesting case the interval rewrite exists for.
    const { scale } = this.placement
    const toEm = (p: Pt2): Pt2 => [p[0] / scale, -p[1] / scale]
    const glyphBoxes = glyphBoundingBoxes(this.placement.contours)
    const glyphContours = glyphContoursByIndex(this.placement.contours)
    const glyphIndexOf: number[] = []
    const ambiguousAt: boolean[] = []
    for (const b of outerBoundaries) {
      if (b.length < 3) {
        glyphIndexOf.push(-1)
        ambiguousAt.push(false)
        continue
      }
      const probe = interiorPoint(b.map(toEm))
      const { index, ambiguous } = glyphAttribution(probe, glyphContours, glyphBoxes)
      glyphIndexOf.push(index)
      ambiguousAt.push(ambiguous)
    }
    const selected = selectFillRegions(
      regionIds,
      outerBoundaries,
      glyphIndexOf,
      MIN_FEATURE_AREA_M2,
      ambiguousAt,
    )

    if (selected.length === 0) {
      this.onToast('Nothing to place — the text has no visible strokes at this size.')
      return
    }

    try {
      const instance = this.wasmScene.place_text(
        sketch,
        BigUint64Array.from(selected),
        this.placement.depth,
        this.placement.name,
        this._activeGroup ?? undefined,
      )
      this.onPlaced(instance)
    } catch (err) {
      this._toastError(err)
    }
  }

  private _toastError(err: unknown): void {
    const code = parseKernelErrorCode(err)
    const rawMsg = err instanceof Error ? err.message : String(err)
    this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      if (this.idlePlaneLock !== null) {
        this.idlePlaneLock = null
        return
      }
      this.cancel()
      return
    }
    if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
      this.idlePlaneLock = nextIdlePlaneLock(this.idlePlaneLock, ev.key)
    }
  }

  cancel(): void {
    this.idlePlaneLock = null
    this._lastHoverPlane = null
    clearPreview(this.preview)
  }
}
