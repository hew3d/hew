/**
 * sheetDetents — pure height math for the Parts sheet's iOS-style bottom
 * sheet (PartsSheet.tsx): three detents (peek/half/full) plus the drag math
 * that lets a pointer-driven grab-handle drag track 1:1 and settle on the
 * nearest one on release. UI-free so the snap behavior is unit-testable
 * without mounting anything.
 *
 * Landscape used to render a wholly different left-edge side sheet (tab <->
 * 340px panel) sharing this same `detent` state — Kurt's on-device playtest
 * called for removing that composition entirely: landscape now uses the
 * SAME bottom sheet as portrait (PartsSheet.tsx, width-capped and centered),
 * so the side-sheet-only helpers that used to live below (`SIDE_SHEET_*`,
 * `isSideSheetOpen`, `sideSheetWidthPx`, `toggledSideDetent`) are gone —
 * `detentHeightPx`/`clampDragHeightPx`/`nearestDetent` above are unchanged
 * and now drive BOTH orientations' height math unmodified (landscape's
 * shorter viewport just yields smaller half/full pixel heights from the
 * same fractions).
 */

export type SheetDetent = 'peek' | 'half' | 'full'

/** Peek height (px): just the grab handle + part-count line — fixed,
 *  content-driven, not proportional to screen size (unlike half/full). */
export const PEEK_HEIGHT_PX = 64

/** Fraction of the viewport container's height the "half" and "full"
 *  detents occupy. Full deliberately stops short of 100% (the iOS sheet
 *  convention) so a sliver of the viewport stays visible/tappable behind
 *  it — the top strip is a separate flex row above this container, so 92%
 *  here never reaches under it. */
const HALF_FRACTION = 0.5
const FULL_FRACTION = 0.92

/** The sheet's height (px) at `detent`, given the viewport container's
 *  current height. */
export function detentHeightPx(detent: SheetDetent, containerHeightPx: number): number {
  if (detent === 'peek') return PEEK_HEIGHT_PX
  if (detent === 'half') return containerHeightPx * HALF_FRACTION
  return containerHeightPx * FULL_FRACTION
}

/** Clamp an in-progress drag height to the [peek, full] range — dragging
 *  past either end just stalls at that detent's height rather than
 *  overshooting past it (a plain pointer drag doesn't rubber-band the way
 *  a native iOS sheet's scroll-physics drag can). */
export function clampDragHeightPx(heightPx: number, containerHeightPx: number): number {
  const min = detentHeightPx('peek', containerHeightPx)
  const max = detentHeightPx('full', containerHeightPx)
  return Math.min(Math.max(heightPx, min), max)
}

/** The nearest detent to `heightPx`, for snapping on drag release. Ties
 *  (equidistant between two detents) resolve to whichever is checked
 *  first in `['peek', 'half', 'full']` order — i.e. the SMALLER detent —
 *  since the scan only replaces the best match on a STRICTLY smaller
 *  distance. */
export function nearestDetent(heightPx: number, containerHeightPx: number): SheetDetent {
  const detents: SheetDetent[] = ['peek', 'half', 'full']
  let best: SheetDetent = 'peek'
  let bestDist = Infinity
  for (const d of detents) {
    const dist = Math.abs(detentHeightPx(d, containerHeightPx) - heightPx)
    if (dist < bestDist) {
      bestDist = dist
      best = d
    }
  }
  return best
}
