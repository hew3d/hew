/**
 * gridMeasure — measures the Library grid's ACTUAL rendered column count, so
 * arrow-key navigation matches whatever the responsive `auto-fill` grid
 * happens to be showing at the moment (playtest finding: fixed 3/4-column
 * assumptions broke Up/Down as soon as the grid wrapped to a different
 * column count than the code assumed).
 *
 * Split into a pure function (`columnCountFromOffsets`, real logic, unit
 * tested directly on plain numbers) and a thin DOM wrapper
 * (`measureGridColumns`) — jsdom has no layout engine, so every child's
 * `offsetTop` reads 0 there; component tests mock this module instead of
 * relying on the DOM wrapper (see LibraryDialog.test.tsx).
 */

/** Given each grid child's `offsetTop` in document order, the number of
 * tiles sharing the first row's offset — i.e. the rendered column count.
 * Falls back to 1 for an empty grid so callers never divide by zero or
 * produce a zero/NaN move stride. */
export function columnCountFromOffsets(offsetTops: number[]): number {
  if (offsetTops.length === 0) return 1
  const firstTop = offsetTops[0]
  const count = offsetTops.filter((t) => t === firstTop).length
  return count > 0 ? count : 1
}

/** Reads `container`'s direct children's `offsetTop` and reduces them to a
 * column count. `null`/empty containers measure as 1 column. */
export function measureGridColumns(container: HTMLElement | null): number {
  if (!container) return 1
  const offsets = Array.from(container.children).map((el) => (el as HTMLElement).offsetTop)
  return columnCountFromOffsets(offsets)
}
