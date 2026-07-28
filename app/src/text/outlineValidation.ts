/**
 * outlineValidation — self-intersecting glyph outline detection
 * (docs/design/3d-text-fonts.md's "Outline quality — warn, don't silently
 * mangle" section).
 *
 * `flatten.ts`'s `classifyContourFill`/`contourNestingDepth` are an exact
 * decision procedure for NON-CROSSING contours (glyph outlines are assumed
 * to be simple loops that only nest, never cross themselves). Opening the
 * font picker to arbitrary system fonts makes a self-crossing default
 * instance — the common failure mode this app's bundled-font screening
 * exists to dodge (see `text/fonts.ts`'s `BUNDLED_MANIFEST` doc comment) —
 * the common case rather than a hand-picked exception. This module answers
 * a narrower, purely geometric question those functions don't: does ANY
 * single contour cross itself? That is never correct glyph geometry (a
 * simple loop's boundary never crosses itself, even though two SEPARATE
 * loops routinely nest — an 'O's outer ring and its counter), so a "yes"
 * here is always worth a warning, regardless of which family or weight
 * produced it.
 *
 * Deliberately never used to block or silently substitute a font
 * (DEVELOPMENT.md rule 4's spirit, applied app-side, restated in the design
 * doc): the sliver/area guards in `TextPlaceTool.ts` and the kernel's own
 * region resolution already cope with a pinched/self-touching contour (see
 * that file's top-of-file doc comment), so a flagged glyph can still be
 * placed — this only gives the user an honest heads-up when the result
 * might not look like the flagged glyph's ink.
 */

import type { Font } from 'opentype.js'
import { dedupeContourPoints, pathCommandsToContours, sagittaToleranceEm, type FlattenCommand, type Pt2 } from './flatten'

/** One glyph whose flattened outline contains a self-intersecting (or
 *  self-touching, treated the same way — see `contourSelfIntersects`)
 *  contour. */
export interface GlyphOutlineWarning {
  /** The character whose glyph is affected. */
  char: string
}

/** Orientation of the ordered triple `(p, q, r)`: positive for
 *  counter-clockwise, negative for clockwise, zero for collinear. Standard
 *  2D cross-product sign test, shared by `segmentsCross`'s general and
 *  collinear-overlap cases. */
function orientation(p: Pt2, q: Pt2, r: Pt2): number {
  const val = (q[1] - p[1]) * (r[0] - q[0]) - (q[0] - p[0]) * (r[1] - q[1])
  if (Math.abs(val) < 1e-9) return 0
  return val > 0 ? 1 : -1
}

/** True iff `q` lies within the axis-aligned bounding box of segment `p`-`r`
 *  — the standard collinear-overlap companion to `orientation`'s zero case
 *  (only ever called when `p`, `q`, `r` are already known collinear). */
function onSegment(p: Pt2, q: Pt2, r: Pt2): boolean {
  return (
    Math.min(p[0], r[0]) - 1e-9 <= q[0] &&
    q[0] <= Math.max(p[0], r[0]) + 1e-9 &&
    Math.min(p[1], r[1]) - 1e-9 <= q[1] &&
    q[1] <= Math.max(p[1], r[1]) + 1e-9
  )
}

/**
 * True iff closed segments `p1`-`p2` and `p3`-`p4` cross or touch anywhere
 * (the general orientation-sign test, plus the four collinear-touching
 * special cases). Callers only ever invoke this on NON-ADJACENT edges of
 * the same contour (see `contourSelfIntersects`) — a shared endpoint
 * between ADJACENT edges is normal polygon structure, not a crossing, and
 * is excluded before this function ever sees the pair.
 */
function segmentsCross(p1: Pt2, p2: Pt2, p3: Pt2, p4: Pt2): boolean {
  const o1 = orientation(p1, p2, p3)
  const o2 = orientation(p1, p2, p4)
  const o3 = orientation(p3, p4, p1)
  const o4 = orientation(p3, p4, p2)

  if (o1 !== o2 && o3 !== o4) return true

  if (o1 === 0 && onSegment(p1, p3, p2)) return true
  if (o2 === 0 && onSegment(p1, p4, p2)) return true
  if (o3 === 0 && onSegment(p3, p1, p4)) return true
  if (o4 === 0 && onSegment(p3, p2, p4)) return true

  return false
}

/**
 * True iff `contour` (a closed polygon loop, points in order) crosses or
 * touches itself anywhere other than at consecutive shared vertices —
 * exactly the "self-intersecting contour" case `classifyContourFill` cannot
 * decide. Every edge is checked against every NON-ADJACENT edge (adjacent
 * edges — including the wraparound pair between the last and first edge —
 * always share exactly one endpoint by construction, which is ordinary
 * closed-polygon structure, not a self-crossing, so those pairs are
 * skipped rather than flagged). O(n²) in the contour's point count, which
 * is fine at glyph scale (tens of points per contour, never thousands).
 */
export function contourSelfIntersects(contour: readonly Pt2[]): boolean {
  const n = contour.length
  if (n < 4) return false // a triangle (or smaller) can never self-cross
  for (let i = 0; i < n; i++) {
    const a1 = contour[i]
    const a2 = contour[(i + 1) % n]
    // j starts at i+2 (skip the edge itself and its immediate successor,
    // which shares endpoint a2) and stops before the wraparound edge that
    // shares endpoint a1 with edge i (j === n-1 when i === 0).
    for (let j = i + 2; j < n; j++) {
      if (i === 0 && j === n - 1) continue // wraparound-adjacent pair
      const b1 = contour[j]
      const b2 = contour[(j + 1) % n]
      if (segmentsCross(a1, a2, b1, b2)) return true
    }
  }
  return false
}

/**
 * Flattens every glyph needed for the distinct, non-whitespace characters
 * in `text` (deduplicated — a repeated letter is only checked once) and
 * reports which ones contain a self-intersecting contour, at the same
 * flattening tolerance (`sagittaToleranceEm`, scaled for `heightMeters`)
 * the real placement pipeline (`text/glyphRun.ts`) would use for the same
 * text and height — so a warning reflects the geometry that would actually
 * be extruded, not a coarser or finer approximation of it. A glyph with no
 * outline at all (space, a font's `.notdef` for an unsupported character)
 * contributes no contours and is never flagged. Order of the returned
 * warnings follows `text`'s own first-occurrence order.
 */
export function validateGlyphOutlines(font: Font, text: string, heightMeters: number): GlyphOutlineWarning[] {
  const tolEm = sagittaToleranceEm(heightMeters, font.unitsPerEm)
  const seen = new Set<string>()
  const warnings: GlyphOutlineWarning[] = []
  for (const char of text) {
    if (seen.has(char) || /\s/.test(char)) continue
    seen.add(char)
    const glyph = font.charToGlyph(char)
    if (!glyph) continue
    const path = glyph.getPath(0, 0, font.unitsPerEm)
    const commands = path.commands as unknown as FlattenCommand[]
    // Dedupe only — deliberately NOT `weldContour`, whose additional
    // near-zero-net-area drop would silently exempt a self-crossing
    // "bowtie" contour from the self-intersection check below whenever its
    // two lobes happen to cancel to zero net signed area (adversarial-
    // review finding 3; see `dedupeContourPoints`'s doc comment).
    const contours = pathCommandsToContours(commands, tolEm)
      .map((c) => dedupeContourPoints(c))
      .filter((c) => c.length >= 3)
    if (contours.some((c) => contourSelfIntersects(c))) {
      warnings.push({ char })
    }
  }
  return warnings
}
