/**
 * LibraryListHead — the List view's column header row (playtest round-3
 * finding #3: a real Finder/Explorer-style list — exactly Name/Type/Size,
 * each sortable and resizable).
 *
 * Sorting: clicking a header cycles that column asc → desc → asc; only one
 * column is ever active. Resizing: a draggable divider sits between Name/
 * Type and between Type/Size — dragging one resizes the column to its own
 * left (the standard file-manager convention), so Size, which is last and
 * has no divider of its own, still visibly resizes as a RESULT of the other
 * two. Column widths are the one piece of List-view state that outlives the
 * session (sort order does not — it's plain view-local state, reset on
 * reopen).
 */

import { useEffect, useRef } from 'react'
import type { ListSortColumn, ListSortDirection } from '../../library/libraryModel'

/** Only Name and Type carry an explicit persisted width — Size is the
 * flexible remainder (`flex: 1 1 0`), matching how Finder/Explorer give the
 * trailing column whatever space is left rather than a fixed one. */
export interface LibraryListColWidths {
  name: number
  type: number
}

export const LIST_COL_MIN: LibraryListColWidths = { name: 140, type: 70 }
const LIST_COL_DEFAULT: LibraryListColWidths = { name: 220, type: 92 }

const STORAGE_KEY = 'hew.library.listCols'

/** Try/catch every access — same convention as `LibraryDialog`'s own
 * `hew.library.view` persistence: privacy mode or a disabled storage
 * backend degrades to the built-in default instead of throwing. A stored
 * width below the current minimum (an older build with a smaller floor, or
 * hand-edited storage) is treated as absent rather than trusted verbatim. */
export function loadListColWidths(): LibraryListColWidths {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw === null) return { ...LIST_COL_DEFAULT }
    const parsed = JSON.parse(raw) as Partial<LibraryListColWidths>
    const name = typeof parsed.name === 'number' && parsed.name >= LIST_COL_MIN.name ? parsed.name : LIST_COL_DEFAULT.name
    const type = typeof parsed.type === 'number' && parsed.type >= LIST_COL_MIN.type ? parsed.type : LIST_COL_DEFAULT.type
    return { name, type }
  } catch {
    return { ...LIST_COL_DEFAULT }
  }
}

/** Pure resize math: the divider's pointer moved `deltaX` px from where the
 * drag started — the column's next width, floored at `min`. Split out and
 * exported so the actual clamping logic is unit-testable directly (see
 * LibraryListHead.test.ts) — jsdom has no `PointerEvent` implementation
 * (same gap `viewport/fovDrag.ts`/`zoomWindowDrag.ts` work around), so a
 * real drag can't be simulated in a component test here. */
export function nextColumnWidth(startWidth: number, deltaX: number, min: number): number {
  return Math.max(min, startWidth + deltaX)
}

export function persistListColWidths(widths: LibraryListColWidths): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(widths))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
}

export interface LibraryListHeadProps {
  widths: LibraryListColWidths
  sortColumn: ListSortColumn
  sortDirection: ListSortDirection
  onSort(column: ListSortColumn): void
  onResize(column: keyof LibraryListColWidths, width: number): void
}

const COLUMNS: { id: ListSortColumn; label: string }[] = [
  { id: 'name', label: 'Name' },
  { id: 'type', label: 'Type' },
  { id: 'size', label: 'Size' },
]

export function LibraryListHead({ widths, sortColumn, sortDirection, onSort, onResize }: LibraryListHeadProps) {
  // Drag state lives in a ref rather than React state — a resize fires many
  // pointermove events per drag, and only the width itself (surfaced through
  // `onResize`) needs to reach React; re-rendering this component on every
  // event would be wasted work.
  const dragRef = useRef<{ column: keyof LibraryListColWidths; startX: number; startWidth: number } | null>(null)
  // The active drag's own teardown (removes the three window listeners
  // below) — kept alongside `dragRef` so it can be invoked from OUTSIDE the
  // pointer-event handlers too. Without this, closing the dialog mid-drag
  // (Escape) unmounts this component while its window listeners are still
  // live: nothing ever fires `onUp` for them, so bare mouse movement after
  // the dialog is gone keeps calling `onResize` into a component that no
  // longer exists. The unmount effect below runs it unconditionally.
  const teardownRef = useRef<(() => void) | null>(null)

  function ariaSort(id: ListSortColumn): 'ascending' | 'descending' | 'none' {
    if (sortColumn !== id) return 'none'
    return sortDirection === 'asc' ? 'ascending' : 'descending'
  }

  function beginResize(column: keyof LibraryListColWidths, e: React.PointerEvent) {
    e.preventDefault()
    const startWidth = widths[column]
    dragRef.current = { column, startX: e.clientX, startWidth }
    function onMove(ev: PointerEvent) {
      const drag = dragRef.current
      if (!drag) return
      onResize(drag.column, nextColumnWidth(drag.startWidth, ev.clientX - drag.startX, LIST_COL_MIN[drag.column]))
    }
    function onUp() {
      dragRef.current = null
      teardownRef.current = null
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
    }
    teardownRef.current = onUp
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    // A cancelled pointer (system gesture, window drag-out) must end the
    // resize too, or bare pointer movement keeps resizing forever.
    window.addEventListener('pointercancel', onUp)
  }

  // This component only ever exists while the dialog is open AND showing
  // List view (its parent renders it conditionally) — closing the dialog or
  // switching to Grid view unmounts it for real, so a plain unmount cleanup
  // is enough to catch every case (see `teardownRef`'s doc comment above).
  useEffect(() => {
    return () => teardownRef.current?.()
  }, [])

  return (
    <div className="hwlib__list-head" role="row">
      {COLUMNS.map((col, i) => (
        <div
          key={col.id}
          role="columnheader"
          aria-sort={ariaSort(col.id)}
          className={`hwlib__list-head-cell hwlib__list-head-cell--${col.id}`}
          style={col.id === 'size' ? { flex: '1 1 0', minWidth: LIST_COL_MIN.type } : { flex: `0 0 ${widths[col.id]}px`, width: widths[col.id] }}
        >
          <button type="button" className="hwlib__list-sort-btn" onClick={() => onSort(col.id)}>
            {col.label}
            {sortColumn === col.id && (
              <span className="hwlib__list-sort-glyph" aria-hidden="true">
                {sortDirection === 'asc' ? '▲' : '▼'}
              </span>
            )}
          </button>
          {i < COLUMNS.length - 1 && (
            <span
              className="hwlib__list-col-resize"
              role="separator"
              aria-orientation="vertical"
              aria-label={`Resize ${col.label} column`}
              onPointerDown={(e) => beginResize(col.id as 'name' | 'type', e)}
            />
          )}
        </div>
      ))}
    </div>
  )
}
