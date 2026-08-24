/**
 * LibraryDialog — the modal Library browser (design handoff frames 1a
 * Components grid, 1b Materials grid, 1f Manage). Lists every `.hew` item
 * in the library folder (`../io/libraryStore`), grouped by category
 * (component/material/model — `../library/libraryModel`'s derivation),
 * with search, scope filters (All / In this model / Recently saved),
 * user collections, a detail pane that reads the manifest only (never
 * geometry), and full manage actions (rename, keywords, collection,
 * re-render thumbnail, reveal, delete) reachable from both the tile's `⋯`
 * menu and the detail pane directly — no action requires the menu, and
 * none requires right-click (settled design decisions #2, #5).
 *
 * This component only renders the browser and reports intent through its
 * props — it does not itself insert geometry, arm the Paint tool, or touch
 * the open document beyond the read-only `placements`/`materialInPalette`
 * lookups. Wiring those actions into the live document is the caller's job
 * (a later integration wave, per the effort's file-ownership split).
 */

import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import { libraryStore } from '../io/libraryStore'
import {
  readItemAsset,
  readItemSummary,
  renderItemThumbnail,
  sha256Hex,
  updateItemMeta,
} from '../library/itemFiles'
import {
  buildLibraryItem,
  collectionTreeFromPaths,
  erroredItem,
  filterItems,
  normalizeCollectionPath,
  sortListItems,
  type ListSortColumn,
  type ListSortDirection,
  type LibraryScope,
} from '../library/libraryModel'
import type { LibraryCategory, LibraryFileEntry, LibraryItem } from '../library/types'
import { measureGridColumns } from './library/gridMeasure'
import { GridViewGlyph, ListViewGlyph, CloseGlyph, SearchGlyph } from './library/icons'
import { LibraryDetailPane } from './library/LibraryDetailPane'
import {
  LibraryListHead,
  loadListColWidths,
  nextColumnWidth,
  persistListColWidths,
  type LibraryListColWidths,
} from './library/LibraryListHead'
import { LibraryListRow } from './library/LibraryListRow'
import { LibraryMenu, revealLabel } from './library/LibraryMenu'
import { LibraryTile, tileElementId } from './library/LibraryTile'

/** Persisted grid/list view choice — one key, shared by every window that
 * opens the browser (same convention as `settings/units.ts`'s
 * `hew.settings.lengthUnit`: try/catch every access, privacy mode or a
 * disabled storage backend degrades to the in-memory default instead of
 * throwing). */
const VIEW_STORAGE_KEY = 'hew.library.view'
type LibraryView = 'grid' | 'list'

function loadInitialView(): LibraryView {
  try {
    return localStorage.getItem(VIEW_STORAGE_KEY) === 'list' ? 'list' : 'grid'
  } catch {
    return 'grid'
  }
}

function persistView(view: LibraryView): void {
  try {
    localStorage.setItem(VIEW_STORAGE_KEY, view)
  } catch {
    /* ignore quota / privacy-mode errors */
  }
}

/** The three body columns' user-resizable widths (playtest round-4 finding
 * #1): the sidebar and detail pane drag Finder-style, exactly the same
 * pointer-capture pattern `LibraryListHead` already uses for its own
 * column resize (`nextColumnWidth`, imported above, is the same clamp
 * primitive). The center column has no width of its own — it's always
 * `flex: 1 1 auto`, the remainder between the other two. Persisted the same
 * way as the List view's column widths: one JSON blob, try/catch every
 * access. */
export interface LibraryColWidths {
  sidebar: number
  detail: number
}

export const COL_WIDTH_MIN: LibraryColWidths = { sidebar: 160, detail: 220 }
/** Hard ceiling for a PERSISTED column width (playtest round-4 finding #1c)
 * — defense against hand-edited/corrupted `localStorage`, layered under the
 * CSS `max-width: 40%` on `.hwlib__sidebar`/`.hwlib__detail` (the layer that
 * protects live layout at every window size) and the live drag clamp in
 * `beginColResize` below (the layer that protects a drag in progress). Well
 * under half of the dialog's own max width (`min(920px, 92vw)`), so nothing
 * the UI itself can ever produce ends up capped here — only a value no
 * legitimate drag could have written. */
export const COL_WIDTH_MAX: LibraryColWidths = { sidebar: 400, detail: 400 }
const COL_WIDTH_DEFAULT: LibraryColWidths = { sidebar: 168, detail: 236 }
const COL_WIDTHS_STORAGE_KEY = 'hew.library.colWidths'

/** A stored width below the current minimum (an older build with a smaller
 * floor, or hand-edited storage) is treated as absent rather than trusted
 * verbatim, same convention as `loadListColWidths`. A stored width ABOVE
 * the hard max is capped rather than rejected outright — it's still a
 * perfectly usable width, just not one the UI would ever produce on its
 * own, so there's no reason to discard the user's stored preference
 * entirely over it (finding #1c). */
function clampStoredColWidth(value: unknown, min: number, max: number, fallback: number): number {
  if (typeof value !== 'number' || value < min) return fallback
  return Math.min(value, max)
}

export function loadColWidths(): LibraryColWidths {
  try {
    const raw = localStorage.getItem(COL_WIDTHS_STORAGE_KEY)
    if (raw === null) return { ...COL_WIDTH_DEFAULT }
    const parsed = JSON.parse(raw) as Partial<LibraryColWidths>
    const sidebar = clampStoredColWidth(parsed.sidebar, COL_WIDTH_MIN.sidebar, COL_WIDTH_MAX.sidebar, COL_WIDTH_DEFAULT.sidebar)
    const detail = clampStoredColWidth(parsed.detail, COL_WIDTH_MIN.detail, COL_WIDTH_MAX.detail, COL_WIDTH_DEFAULT.detail)
    return { sidebar, detail }
  } catch {
    return { ...COL_WIDTH_DEFAULT }
  }
}

export function persistColWidths(widths: LibraryColWidths): void {
  try {
    localStorage.setItem(COL_WIDTHS_STORAGE_KEY, JSON.stringify(widths))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
}

/** The live half of finding #1's fix: clamps a body column's NEXT width
 * (already floored at `min` by `nextColumnWidth`) so the CENTER column left
 * over between the sidebar and the detail pane never drops below
 * `centerMin` — this is what keeps an in-progress DRAG from wedging the
 * dialog, complementing the CSS `max-width: 40%` on `.hwlib__sidebar`/
 * `.hwlib__detail` (which protects a bad PERSISTED value and every window
 * size, but can't see a drag that hasn't committed to state yet) and
 * `loadColWidths`'s hard-max clamp (which only ever runs once, on mount).
 * `containerWidth <= 0` — not yet laid out, which is what jsdom's no-op
 * layout engine reports for every element — disables the live clamp rather
 * than collapsing the dragged column to its minimum, so component tests
 * that can't produce a real layout still see plain `nextColumnWidth`
 * behavior. Pure and exported so the math is unit-testable directly (see
 * LibraryDialog.test.tsx) — same convention as `nextColumnWidth` itself. */
export function clampColWidthLive(
  desired: number,
  min: number,
  otherColumnWidth: number,
  containerWidth: number,
  dividersWidth: number,
  centerMin: number,
): number {
  if (containerWidth <= 0) return Math.max(min, desired)
  const maxAllowed = Math.max(min, containerWidth - otherColumnWidth - dividersWidth - centerMin)
  return Math.min(Math.max(min, desired), maxAllowed)
}

/** Combined width of the two `.hwlib__col-resize` dividers (5px each in the
 * stylesheet below) — both sit between the live-measured body width and the
 * sidebar/detail/center columns it's split across. */
const COL_DIVIDERS_WIDTH = 10
/** The center column's own floor during a live drag (part b of finding
 * #1's fix) — comfortably enough to keep the grid/list usable and both
 * resize dividers reachable, well short of the CSS `max-width: 40%`
 * ceiling on the other two columns. */
const CENTER_COL_MIN_WIDTH = 200

export interface LibraryDialogProps {
  open: boolean
  onClose(): void
  /** hew.library source_id → live placement count in the open document. */
  placements: Record<string, number>
  /** Insert a component/model item (closes the modal; App arms cursor placement). */
  onInsert(item: LibraryItem, bytes: Uint8Array): void
  /** Open the item as its own document. */
  onOpenAsDocument(item: LibraryItem, bytes: Uint8Array): void
  /** Material primary action: copy into palette + arm Paint (App closes modal). */
  onPaintWith(item: LibraryItem, bytes: Uint8Array): void
  /** Material secondary action: copy into palette, keep the modal open. */
  onAddToPalette(item: LibraryItem, bytes: Uint8Array): void
  /** Optional: which of this material item's palette entries already exist in the doc. */
  materialInPalette?(bytes: Uint8Array): Promise<boolean>
  /**
   * How the browser is hosted. `'modal'` (default): the web build's in-app
   * overlay — scrim, fixed size, z-2000 band. `'window'`: the content of a
   * REAL native window (the desktop shell's resizable Library window) — no
   * scrim, fills the window, and `onClose` closes that window.
   */
  variant?: 'modal' | 'window'
}

/** The sidebar's "All" row (finding #4) lists every item at once — its
 * default-action dispatch still reads each ITEM's own category (see
 * `runDefaultAction`/`runOtherAction`, which only ever look at
 * `item.category`, never this sidebar selection), so mixing categories in
 * one grid never confuses which verb a given tile's Enter/double-click
 * runs. */
const CATEGORIES: { value: LibraryCategory | 'all'; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'component', label: 'Components' },
  { value: 'material', label: 'Materials' },
  { value: 'model', label: 'Models' },
]

const SCOPES: { value: LibraryScope; label: string }[] = [
  { value: 'all', label: 'All items' },
  { value: 'in-model', label: 'In this model' },
  { value: 'recent', label: 'Recently saved' },
]

const CATEGORY_LABEL: Record<LibraryCategory | 'all', string> = {
  all: 'All',
  component: 'Components',
  material: 'Materials',
  model: 'Models',
}

/** Lower-case, grammatically-plain form for the empty-category sentence
 * ("No components here yet.") — `CATEGORY_LABEL['all'].toLowerCase()` would
 * read "No all here yet.", so "All" gets its own word here instead. */
const CATEGORY_LABEL_LOWER: Record<LibraryCategory | 'all', string> = {
  all: 'items',
  component: 'components',
  material: 'materials',
  model: 'models',
}

/** The category-default action's short verb, for the tile's hover button and
 * the footer's keyboard hint. */
function defaultActionLabel(item: LibraryItem): string {
  if (item.category === 'component') return 'Insert'
  if (item.category === 'model') return 'Open'
  return 'Paint'
}

/** The "other" action (mod+Enter) — always the category's non-default verb. */
function otherActionLabel(item: LibraryItem): string {
  if (item.category === 'component') return 'Open'
  if (item.category === 'model') return 'Insert'
  return 'Add to Palette'
}

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

/** "1 item" / "N items" — the footer count. */
function itemCountLabel(n: number): string {
  return n === 1 ? '1 item' : `${n} items`
}

export function LibraryDialog({
  open,
  onClose,
  placements,
  onInsert,
  onOpenAsDocument,
  onPaintWith,
  onAddToPalette,
  materialInPalette,
  variant = 'modal',
}: LibraryDialogProps) {
  const [items, setItems] = useState<LibraryItem[]>([])
  const [loading, setLoading] = useState(true)
  const [unavailable, setUnavailable] = useState(false)
  // A bound web folder waiting on a permission re-grant (webLibraryStore's
  // needsReconnect): the grid shows a Reconnect button whose click is the
  // user gesture requestPermission needs.
  const [needsReconnect, setNeedsReconnect] = useState(false)
  // The Reconnect click was refused (browser denied the re-grant) — shown
  // under the button so the click never silently no-ops.
  const [reconnectDenied, setReconnectDenied] = useState(false)
  // The listing itself failed (storage rejected, not just empty) — shown in
  // the grid instead of a false "save something to get started".
  const [loadError, setLoadError] = useState<string | null>(null)
  const [reloadToken, setReloadToken] = useState(0)
  const [folderPath, setFolderPath] = useState<string | null>(null)

  const [category, setCategory] = useState<LibraryCategory | 'all'>('component')
  const [query, setQuery] = useState('')
  const [scope, setScope] = useState<LibraryScope>('all')
  const [selectedCollection, setSelectedCollection] = useState<string | null>(null)
  const [localCollections, setLocalCollections] = useState<string[]>([])
  const [newCollectionInput, setNewCollectionInput] = useState<string | null>(null)
  const [view, setView] = useState<LibraryView>(loadInitialView)
  // List view sort — view-local, NOT persisted (only the column widths
  // below are; see `LibraryListHead`'s doc comment).
  const [sortColumn, setSortColumn] = useState<ListSortColumn>('name')
  const [sortDirection, setSortDirection] = useState<ListSortDirection>('asc')
  const [listColWidths, setListColWidths] = useState<LibraryListColWidths>(loadListColWidths)
  // Body column widths (sidebar/detail — finding #1). Drag state lives in a
  // ref rather than React state, same reasoning as `LibraryListHead`'s own
  // `dragRef`: a resize fires many pointermove events per drag, and only
  // the width itself needs to reach React.
  const [colWidths, setColWidths] = useState<LibraryColWidths>(loadColWidths)
  const colDragRef = useRef<{ column: keyof LibraryColWidths; startX: number; startWidth: number } | null>(null)
  // The active body-column drag's own teardown (removes the three window
  // listeners `beginColResize` adds) — kept alongside `colDragRef` so it can
  // be invoked from OUTSIDE the pointer-event handlers too (finding #3):
  // Escape closes the dialog (setting `open` false) without ever firing a
  // pointerup, so without this the drag's window listeners stay live and
  // keep mutating + persisting `colWidths` from bare mouse movement after
  // the dialog is gone. See the effect below.
  const colResizeTeardownRef = useRef<(() => void) | null>(null)
  // The `.hwlib__body` row (sidebar/center/detail) — measured live during a
  // drag so `clampColWidthLive` can keep the center column from collapsing
  // at whatever size the WINDOW actually is right now, not just whatever
  // `colWidths` happened to add up to before the drag started.
  const bodyRef = useRef<HTMLDivElement>(null)

  const [selectedFileName, setSelectedFileName] = useState<string | null>(null)
  const [menuFor, setMenuFor] = useState<{ fileName: string; anchor: { top: number; left: number; bottom: number } } | null>(null)
  const [deleteConfirmFor, setDeleteConfirmFor] = useState<string | null>(null)
  const [inPaletteMap, setInPaletteMap] = useState<Record<string, boolean>>({})
  // Surfaces the last failed manage mutation (rename/keyword/collection/
  // re-render/reveal/delete) — these otherwise fail silently with no path
  // back to the user. Cleared on the next successful mutation or selection
  // change (see the dedicated effect below).
  const [actionError, setActionError] = useState<string | null>(null)

  // Bytes are cached per file name (needed for every action callback); manage
  // mutations re-read the current bytes from the store instead (see
  // `persistMeta`) so a concurrent external edit isn't clobbered.
  const bytesRef = useRef<Map<string, Uint8Array>>(new Map())
  // Object URLs keyed by content hash (thumbnails) / file name (texture
  // previews); revoked on unmount (see the dedicated effect below), and the
  // texture cache is also revoked+cleared on any external library-folder
  // change (see `clearTextureCache` below) since it's keyed by file name,
  // not content hash, and would otherwise show a stale image after an
  // external edit reuses the same name.
  const thumbUrlsRef = useRef<Map<string, string>>(new Map())
  const textureUrlsRef = useRef<Map<string, string>>(new Map())
  const [, bumpVersion] = useReducer((n: number) => n + 1, 0)

  // Focus containment (S21): the dialog root for the Tab trap, the grid for
  // aria-activedescendant + returning keyboard focus after a mouse
  // selection, and the search input as the initial-focus target.
  const dialogRef = useRef<HTMLDivElement>(null)
  const gridRef = useRef<HTMLDivElement>(null)
  // The `.hwlib__grid-inner`/`.hwlib__list` child that actually holds the
  // tiles/rows — distinct from `gridRef` (the always-present listbox
  // wrapper, which also renders empty/loading states with no children to
  // measure) since arrow-key nav needs to measure the REAL rendered layout
  // (finding: fixed 3/4-column assumptions broke as soon as the grid
  // wrapped — see `library/gridMeasure.ts`).
  const gridInnerRef = useRef<HTMLDivElement>(null)
  const searchInputRef = useRef<HTMLInputElement>(null)

  const nowMs = useMemo(() => Date.now(), [open])

  function clearTextureCache() {
    for (const url of textureUrlsRef.current.values()) URL.revokeObjectURL(url)
    textureUrlsRef.current.clear()
    bumpVersion()
  }

  // --- Load the listing ------------------------------------------------
  useEffect(() => {
    if (!open) return
    // Drop any texture previews cached in a PRIOR open: they are keyed by
    // file name, and an external edit made while the dialog was closed (the
    // subscribe-driven revoke only runs while open) can reuse a name for new
    // content — so a stale preview would otherwise survive into this open
    // (audit q-web-robustness). Revoking here makes every open re-read.
    clearTextureCache()
    let cancelled = false
    async function load() {
      setLoading(true)
      const store = libraryStore()
      if (!store.available()) {
        if (!cancelled) {
          setUnavailable(true)
          setItems([])
          setLoading(false)
        }
        return
      }
      if (!cancelled) setUnavailable(false)
      try {
        const entries = await store.list()
        const ws = await store.webStorage?.()
        if (!cancelled) {
          setNeedsReconnect(ws?.needsReconnect ?? false)
          setLoadError(null)
        }
        const built: LibraryItem[] = []
        for (const file of entries) {
          try {
            const bytes = await store.read(file.fileName)
            if (cancelled) return
            bytesRef.current.set(file.fileName, bytes)
            const [hash, summary] = await Promise.all([sha256Hex(bytes), readItemSummary(bytes)])
            built.push(buildLibraryItem(file, summary, hash))
          } catch (err) {
            built.push(erroredItem(file, '', err instanceof Error ? err.message : String(err)))
          }
        }
        if (!cancelled) setItems(built)
      } catch (err) {
        // list()/webStorage() themselves rejected (storage failure, a lazy
        // backend that couldn't load) — an empty grid would misreport this
        // as "no items yet", and an uncaught rejection would trip the
        // global reproducer handler.
        if (!cancelled) {
          setItems([])
          setLoadError(describeError(err))
        }
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    void load()
    return () => {
      cancelled = true
    }
  }, [open, reloadToken])

  // --- Mid-session permission loss ---------------------------------------
  // A failed manage mutation may mean the bound folder's permission was
  // revoked while the dialog sat open on stale items; re-probe so the grid
  // swaps to the Reconnect state instead of an error message that points
  // at a button that isn't on screen.
  useEffect(() => {
    if (actionError === null) return
    let cancelled = false
    void libraryStore()
      .webStorage?.()
      .then((ws) => {
        if (!cancelled) setNeedsReconnect(ws.needsReconnect)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [actionError])

  // --- Reload on external library-folder changes ------------------------
  useEffect(() => {
    if (!open) return
    return libraryStore().subscribe(() => {
      // An external change (another window, Settings) may reuse a file name
      // for different content — drop the stale texture cache rather than
      // let a tile/detail pane keep showing the old image under it.
      clearTextureCache()
      setReloadToken((t) => t + 1)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  // --- Folder path (footer) ---------------------------------------------
  useEffect(() => {
    if (!open) return
    let cancelled = false
    libraryStore()
      .folderInfo()
      .then((info) => {
        if (!cancelled) setFolderPath(info.path)
      })
      .catch(() => {
        // The footer path is decorative; leave it unset rather than surface
        // this as a manage-mutation error.
        if (!cancelled) setFolderPath(null)
      })
    return () => {
      cancelled = true
    }
  }, [open, reloadToken])

  // --- Invalidate the texture cache on a folder switch --------------------
  // Belt-and-suspenders alongside the subscribe-triggered clear above: covers
  // a folder change this window itself initiated (Settings) without relying
  // on the subscription round-trip.
  useEffect(() => {
    clearTextureCache()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [folderPath])

  // --- Focus the search input on open (S21: initial focus for a11y) -------
  useEffect(() => {
    if (open) searchInputRef.current?.focus()
  }, [open])

  // --- Persist the grid/list view choice -----------------------------------
  useEffect(() => {
    persistView(view)
  }, [view])

  // --- Persist the List view's column widths --------------------------------
  useEffect(() => {
    persistListColWidths(listColWidths)
  }, [listColWidths])

  // --- Persist the body columns' widths (finding #1) -------------------------
  useEffect(() => {
    persistColWidths(colWidths)
  }, [colWidths])

  // --- Tear down an in-flight column drag when the dialog closes (finding
  // #3) --------------------------------------------------------------------
  // Runs on every `open` transition (both directions) AND on unmount — the
  // cleanup fires before the next render whenever `open` itself changes, so
  // "dialog just closed" and "component just unmounted" are both covered by
  // the one cleanup. Calling `colResizeTeardownRef.current` when no drag is
  // active is a no-op (it's null then).
  useEffect(() => {
    return () => colResizeTeardownRef.current?.()
  }, [open])

  // --- Clear the action-mutation error on selection change ----------------
  useEffect(() => {
    setActionError(null)
  }, [selectedFileName])

  // --- Thumbnails (components/models — materials render their own swatch) ---
  useEffect(() => {
    if (!open) return
    let cancelled = false
    async function loadThumbs() {
      const store = libraryStore()
      for (const item of items) {
        if (item.error || item.category === 'material') continue
        if (thumbUrlsRef.current.has(item.contentHash)) continue
        let png = await store.readThumbnail(item.contentHash)
        if (cancelled) return
        if (png === null) {
          const bytes = bytesRef.current.get(item.file.fileName)
          if (!bytes) continue
          png = await renderItemThumbnail(bytes, 256)
          if (cancelled) return
          if (png !== null) {
            try {
              await store.writeThumbnail(item.contentHash, png)
            } catch {
              /* the thumbnail cache is optional — a failed write (lost
                 folder permission, storage pressure) costs a re-render */
            }
          }
        }
        // Re-check right before creating the URL: `store.writeThumbnail`
        // above is an `await` this loop can be cancelled underneath (reload,
        // unmount) — creating and caching the URL after that would leak it,
        // since the unmount cleanup that revokes everything already tracked
        // runs before this async function resumes.
        if (png !== null && !cancelled) {
          const url = URL.createObjectURL(new Blob([new Uint8Array(png)], { type: 'image/png' }))
          thumbUrlsRef.current.set(item.contentHash, url)
          bumpVersion()
        }
      }
    }
    void loadThumbs()
    return () => {
      cancelled = true
    }
  }, [open, items])

  // --- Material texture previews -----------------------------------------
  useEffect(() => {
    if (!open) return
    let cancelled = false
    async function loadTextures() {
      for (const item of items) {
        if (item.error || item.category !== 'material') continue
        const entry = item.summary.material_entries[0]
        if (!entry || entry.texture_asset === null) continue
        if (textureUrlsRef.current.has(item.file.fileName)) continue
        const bytes = bytesRef.current.get(item.file.fileName)
        if (!bytes) continue
        try {
          const assetBytes = await readItemAsset(bytes, entry.texture_asset)
          // No further `await` between this check and `createObjectURL`
          // below, so nothing can flip `cancelled` in between — unlike the
          // thumbnail effect above, this one was never racy, but the check
          // still has to be the last thing before the URL is created.
          if (cancelled) return
          const mime = /jpe?g/i.test(entry.texture_format ?? '') ? 'image/jpeg' : 'image/png'
          const url = URL.createObjectURL(new Blob([new Uint8Array(assetBytes)], { type: mime }))
          textureUrlsRef.current.set(item.file.fileName, url)
          bumpVersion()
        } catch {
          // Leave it as a color-only fallback rather than crash the tile
          // over a missing/corrupt texture asset.
        }
      }
    }
    void loadTextures()
    return () => {
      cancelled = true
    }
  }, [open, items])

  // --- Revoke every object URL on unmount --------------------------------
  useEffect(() => {
    return () => {
      for (const url of thumbUrlsRef.current.values()) URL.revokeObjectURL(url)
      for (const url of textureUrlsRef.current.values()) URL.revokeObjectURL(url)
      thumbUrlsRef.current.clear()
      textureUrlsRef.current.clear()
    }
  }, [])

  // --- "In palette" resolution for material tiles -------------------------
  useEffect(() => {
    if (!open || materialInPalette === undefined) {
      setInPaletteMap({})
      return
    }
    // Reassigning to a local const carries the `!== undefined` narrowing
    // into the closures below — `materialInPalette` is a function
    // parameter, so TS can't assume it stays defined across a captured
    // closure boundary even though props never change out from under a
    // single render.
    const checkPalette = materialInPalette
    let cancelled = false
    async function check() {
      const rows = await Promise.all(
        items
          .filter((i) => i.category === 'material' && !i.error)
          .map(async (i): Promise<[string, boolean]> => {
            const bytes = bytesRef.current.get(i.file.fileName)
            if (!bytes) return [i.file.fileName, false]
            try {
              return [i.file.fileName, await checkPalette(bytes)]
            } catch {
              return [i.file.fileName, false]
            }
          }),
      )
      if (!cancelled) setInPaletteMap(Object.fromEntries(rows))
    }
    void check()
    return () => {
      cancelled = true
    }
  }, [open, items, materialInPalette])

  // --- Derived listing -----------------------------------------------------
  const visibleItems = useMemo(
    () =>
      filterItems(items, {
        query,
        category,
        scope,
        collection: selectedCollection,
        placements,
        materialInPalette: inPaletteMap,
        nowMs,
      }),
    [items, query, category, scope, selectedCollection, placements, inPaletteMap, nowMs],
  )
  // The List view's Name/Type/Size sort composes with the filtering above —
  // it's a pure reordering of `visibleItems`, applied only while that view
  // is showing (the grid has no column sort). Selection/keyboard nav read
  // from THIS array, not `visibleItems`, so Up/Down always matches whatever
  // order is actually on screen.
  const displayItems = useMemo(
    () => (view === 'list' ? sortListItems(visibleItems, sortColumn, sortDirection) : visibleItems),
    [visibleItems, view, sortColumn, sortDirection],
  )
  const allCollections = useMemo(() => {
    const itemPaths = items.map((i) => i.meta.collection).filter((c): c is string => c !== undefined)
    return collectionTreeFromPaths([...itemPaths, ...localCollections])
  }, [items, localCollections])
  const categoryCounts = useMemo(() => {
    const counts: Record<LibraryCategory | 'all', number> = { all: items.length, component: 0, material: 0, model: 0 }
    for (const item of items) counts[item.category] += 1
    return counts
  }, [items])

  // Keep selection valid as the visible listing changes underneath it —
  // defaults to the first visible tile (frame 1a shows a tile pre-selected).
  useEffect(() => {
    if (displayItems.length === 0) {
      if (selectedFileName !== null) setSelectedFileName(null)
      return
    }
    if (!displayItems.some((i) => i.file.fileName === selectedFileName)) {
      setSelectedFileName(displayItems[0].file.fileName)
    }
  }, [displayItems, selectedFileName])

  const selectedItem = displayItems.find((i) => i.file.fileName === selectedFileName) ?? null
  const menuItem = menuFor !== null ? (items.find((i) => i.file.fileName === menuFor.fileName) ?? null) : null

  function placementCountOf(item: LibraryItem | null): number {
    if (item?.meta.id === undefined) return 0
    return placements[item.meta.id] ?? 0
  }

  // --- Category-default / other action dispatch ---------------------------
  function bytesFor(item: LibraryItem): Uint8Array | undefined {
    return bytesRef.current.get(item.file.fileName)
  }
  function handleInsert(item: LibraryItem) {
    const bytes = bytesFor(item)
    if (bytes) onInsert(item, bytes)
  }
  /** The `'window'` variant's ⋯/right-click menu: a STOCK native context
   * menu via the shell (`../library/nativeChrome`), mapping picks onto the
   * same handlers the web modal's DOM popover uses. Deletion confirms
   * through the native dialog instead of the inline armed button. */
  async function openNativeMenu(item: LibraryItem) {
    setSelectedFileName(item.file.fileName)
    const { popupNativeMenu, nativeConfirm } = await import('../library/nativeChrome')
    // An errored item (unreadable file) can only be revealed or deleted —
    // offering Open/Rename/etc. would silently no-op (adversarial review
    // S13).
    const entries = item.error
      ? [
          ...(canReveal ? [{ id: 'reveal', label: revealLabel }] : []),
          { id: 'delete', label: 'Delete from Library…' },
        ]
      : [
          { id: 'open', label: 'Open as Document' },
          { id: 'rename', label: 'Rename' },
          { id: 'collection', label: 'Add to Collection' },
          { id: 'sep1', label: '', separator: true },
          { id: 'rerender', label: 'Re-render Thumbnail' },
          // Models are their own source, never extracted from another model
          // — Remove Source Info only ever applies to components/materials.
          ...(item.category !== 'model' && item.meta.sourceDoc !== undefined
            ? [{ id: 'removeSourceInfo', label: 'Remove Source Info' }]
            : []),
          ...(canReveal ? [{ id: 'reveal', label: revealLabel }] : []),
          { id: 'sep2', label: '', separator: true },
          { id: 'delete', label: 'Delete from Library…' },
        ]
    const picked = await popupNativeMenu(entries)
    if (picked === 'open') handleOpenAsDocument(item)
    else if (picked === 'rerender') void handleRerenderThumbnail(item)
    else if (picked === 'removeSourceInfo') handleRemoveSourceInfo(item)
    else if (picked === 'reveal') void handleReveal(item)
    else if (picked === 'delete') {
      const ok = await nativeConfirm(
        'Delete from Library',
        `Delete "${item.displayName}" from the library? Documents that already used it are unaffected.`,
        'Delete',
      )
      if (ok) void handleDelete(item)
    }
    // 'rename'/'collection' just select the item — the detail pane's name
    // field and Collection dropdown are always live (edit-in-place).
  }

  function handleOpenAsDocument(item: LibraryItem) {
    const bytes = bytesFor(item)
    if (bytes) onOpenAsDocument(item, bytes)
  }
  function handlePaintWith(item: LibraryItem) {
    const bytes = bytesFor(item)
    if (bytes) onPaintWith(item, bytes)
  }
  function handleAddToPalette(item: LibraryItem) {
    const bytes = bytesFor(item)
    if (bytes) onAddToPalette(item, bytes)
  }
  function runDefaultAction(item: LibraryItem) {
    if (item.category === 'component') handleInsert(item)
    else if (item.category === 'model') handleOpenAsDocument(item)
    else handlePaintWith(item)
  }
  function runOtherAction(item: LibraryItem) {
    if (item.category === 'component') handleOpenAsDocument(item)
    else if (item.category === 'model') handleInsert(item)
    else handleAddToPalette(item)
  }

  // --- Manage mutations -----------------------------------------------------
  async function reloadOne(fileName: string) {
    const bytes = bytesRef.current.get(fileName)
    if (!bytes) return
    const hash = await sha256Hex(bytes)
    const file: LibraryFileEntry = { fileName, size: bytes.length, mtimeMs: Date.now() }
    let updated: LibraryItem
    try {
      const summary = await readItemSummary(bytes)
      updated = buildLibraryItem(file, summary, hash)
    } catch (err) {
      updated = erroredItem(file, hash, err instanceof Error ? err.message : String(err))
    }
    setItems((prev) => prev.map((i) => (i.file.fileName === fileName ? updated : i)))
  }

  async function persistMeta(item: LibraryItem, patch: Record<string, unknown>) {
    try {
      const store = libraryStore()
      // Re-read the CURRENT bytes rather than the ones cached at dialog-open
      // (`bytesRef`) — another window (or an external edit) may have written
      // this file since then, and a stale read-modify-write would silently
      // overwrite that edit.
      const currentBytes = await store.read(item.file.fileName)
      const newBytes = await updateItemMeta(currentBytes, patch)
      await store.write(item.file.fileName, newBytes)
      bytesRef.current.set(item.file.fileName, newBytes)
      // The item's content (hence its thumbnail cache key) just changed —
      // re-rendering is simplest and correct rather than trying to carry the
      // old thumbnail entry over to a new hash.
      const old = thumbUrlsRef.current.get(item.contentHash)
      if (old) {
        URL.revokeObjectURL(old)
        thumbUrlsRef.current.delete(item.contentHash)
      }
      await reloadOne(item.file.fileName)
      setActionError(null)
    } catch (err) {
      setActionError(`Couldn't save changes: ${describeError(err)}`)
    }
  }

  function handleRename(item: LibraryItem, name: string) {
    void persistMeta(item, { name })
  }
  function handleAddKeyword(item: LibraryItem, keyword: string) {
    const kws = Array.from(new Set([...(item.meta.keywords ?? []), keyword]))
    void persistMeta(item, { keywords: kws })
  }
  function handleRemoveKeyword(item: LibraryItem, keyword: string) {
    const kws = (item.meta.keywords ?? []).filter((k) => k !== keyword)
    void persistMeta(item, { keywords: kws })
  }
  function handleChangeCollection(item: LibraryItem, collection: string | null) {
    void persistMeta(item, { collection: collection ?? '' })
  }
  function handleRemoveSourceInfo(item: LibraryItem) {
    void persistMeta(item, { sourceDoc: null })
  }
  async function handleRerenderThumbnail(item: LibraryItem) {
    const bytes = bytesRef.current.get(item.file.fileName)
    if (!bytes) return
    try {
      const png = await renderItemThumbnail(bytes, 256)
      if (png === null) return
      await libraryStore().writeThumbnail(item.contentHash, png)
      const old = thumbUrlsRef.current.get(item.contentHash)
      if (old) URL.revokeObjectURL(old)
      const url = URL.createObjectURL(new Blob([new Uint8Array(png)], { type: 'image/png' }))
      thumbUrlsRef.current.set(item.contentHash, url)
      bumpVersion()
      setActionError(null)
    } catch (err) {
      setActionError(`Couldn't save changes: ${describeError(err)}`)
    }
  }
  /** Web: hand the item's bytes to the user as a plain `.hew` download —
   * browser storage has no Reveal, and this is how items escape the
   * origin-private file system. */
  function handleDownload(item: LibraryItem) {
    const bytes = bytesRef.current.get(item.file.fileName)
    if (bytes === undefined) return
    const blob = new Blob([new Uint8Array(bytes)], { type: 'application/octet-stream' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${item.displayName.replace(/[\\/:]/g, '-')}.hew`
    document.body.appendChild(a)
    a.click()
    a.remove()
    URL.revokeObjectURL(url)
  }

  async function handleReveal(item: LibraryItem) {
    try {
      await libraryStore().reveal(item.file.fileName)
      setActionError(null)
    } catch (err) {
      setActionError(`Couldn't save changes: ${describeError(err)}`)
    }
  }
  /** The detail pane's Delete flow (finding #5): the `'window'` variant
   * confirms through the same native dialog the ⋯ menu already uses
   * (`openNativeMenu` above) instead of arming the inline confirm —
   * `deleteConfirmFor` is left untouched here, so `LibraryDetailPane` never
   * renders its inline `DeleteConfirm` for this variant. The modal/web
   * fallback is unchanged: arm the inline confirm as before. */
  async function requestDelete(item: LibraryItem) {
    if (variant === 'window') {
      const { nativeConfirm } = await import('../library/nativeChrome')
      const ok = await nativeConfirm(
        'Delete from Library',
        `Delete "${item.displayName}" from the library? Documents that already used it are unaffected.`,
        'Delete',
      )
      if (ok) void handleDelete(item)
      return
    }
    setDeleteConfirmFor(item.file.fileName)
  }

  async function handleDelete(item: LibraryItem) {
    try {
      await libraryStore().remove(item.file.fileName)
      bytesRef.current.delete(item.file.fileName)
      const oldThumb = thumbUrlsRef.current.get(item.contentHash)
      if (oldThumb) {
        URL.revokeObjectURL(oldThumb)
        thumbUrlsRef.current.delete(item.contentHash)
      }
      const oldTexture = textureUrlsRef.current.get(item.file.fileName)
      if (oldTexture) {
        URL.revokeObjectURL(oldTexture)
        textureUrlsRef.current.delete(item.file.fileName)
      }
      setItems((prev) => prev.filter((i) => i.file.fileName !== item.file.fileName))
      setDeleteConfirmFor(null)
      setActionError(null)
    } catch (err) {
      setActionError(`Couldn't save changes: ${describeError(err)}`)
    }
  }

  // --- List view: sort + column resize -------------------------------------
  function handleSort(column: ListSortColumn) {
    if (column === sortColumn) {
      setSortDirection((d) => (d === 'asc' ? 'desc' : 'asc'))
    } else {
      setSortColumn(column)
      setSortDirection('asc')
    }
  }
  function handleResizeCol(column: keyof LibraryListColWidths, width: number) {
    setListColWidths((prev) => ({ ...prev, [column]: width }))
  }

  /** Begins dragging one of the two body-column dividers (finding #1),
   * pointer-capture style — the SAME pattern as `LibraryListHead`'s own
   * column resize (pointerdown here, pointermove/up/cancel on `window`).
   * The sidebar divider sits on the sidebar's own right edge (dragging
   * right widens it, exactly `LibraryListHead`'s convention); the detail
   * divider sits on the detail pane's own LEFT edge, so the sign flips —
   * dragging right (into the pane) narrows it. */
  function beginColResize(column: keyof LibraryColWidths, e: React.PointerEvent) {
    e.preventDefault()
    const startWidth = colWidths[column]
    colDragRef.current = { column, startX: e.clientX, startWidth }
    function onMove(ev: PointerEvent) {
      const drag = colDragRef.current
      if (!drag) return
      const rawDelta = ev.clientX - drag.startX
      const deltaX = drag.column === 'sidebar' ? rawDelta : -rawDelta
      const desired = nextColumnWidth(drag.startWidth, deltaX, COL_WIDTH_MIN[drag.column])
      // Functional update so `otherColumnWidth` always reads the CURRENT
      // other column, not whatever it was when this drag started (finding
      // #1b) — the live container width comes from `bodyRef`, measured
      // fresh on every move rather than cached, for the same reason
      // `measureGridColumns` is read fresh on every keydown above.
      setColWidths((prev) => {
        const otherColumnWidth = drag.column === 'sidebar' ? prev.detail : prev.sidebar
        const containerWidth = bodyRef.current?.getBoundingClientRect().width ?? 0
        const width = clampColWidthLive(
          desired,
          COL_WIDTH_MIN[drag.column],
          otherColumnWidth,
          containerWidth,
          COL_DIVIDERS_WIDTH,
          CENTER_COL_MIN_WIDTH,
        )
        return { ...prev, [drag.column]: width }
      })
    }
    function onUp() {
      colDragRef.current = null
      colResizeTeardownRef.current = null
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
    }
    colResizeTeardownRef.current = onUp
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    // A cancelled pointer (system gesture, window drag-out) must end the
    // resize too, or bare pointer movement keeps resizing forever.
    window.addEventListener('pointercancel', onUp)
  }

  function commitNewCollection() {
    const normalized = normalizeCollectionPath(newCollectionInput ?? '')
    if (normalized !== '') {
      setLocalCollections((prev) => (prev.includes(normalized) ? prev : [...prev, normalized]))
      setSelectedCollection(normalized)
    }
    setNewCollectionInput(null)
  }

  // --- Keyboard: grid navigation + Enter/mod+Enter + Escape -----------------
  const moveSelection = useCallback(
    (delta: number) => {
      if (displayItems.length === 0) return
      const idx = displayItems.findIndex((i) => i.file.fileName === selectedFileName)
      const nextIdx = idx === -1 ? 0 : Math.min(displayItems.length - 1, Math.max(0, idx + delta))
      setSelectedFileName(displayItems[nextIdx].file.fileName)
    },
    [displayItems, selectedFileName],
  )

  // Re-registered after every render (no deps array) so it always closes
  // over the current selection/menu/collection-editing state — a modal with
  // this few re-renders per interaction doesn't need the extra bookkeeping
  // a memoized handler + exhaustive deps list would add.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (!open) return
      // The dialog is modal: swallow every keydown that reaches `document`
      // while it's open, so it can never ALSO reach a window-level handler
      // underneath it (Viewport sets tool axis/plane locks off arrow keys;
      // App switches tools off bare letters whenever focus isn't an
      // input/textarea — which includes a focused BUTTON, e.g. this dialog's
      // own tile/menu/confirm buttons). Bubbling runs target → … → document →
      // window, so stopping it here blocks every BUBBLE-phase window
      // listener — the same contract Escape already relied on (see
      // Viewport's `onCtrlKeyDown` comment). The one listener this can't
      // reach is that same `onCtrlKeyDown`: it's registered on window with
      // the CAPTURE flag, so it fires during the capture pass — before the
      // event ever reaches this document-level (bubble-phase) handler — and
      // is documented there as deliberately pure bookkeeping with no
      // preventDefault and no visible side effect, so that gap is safe.
      e.stopPropagation()
      if (e.key === 'Escape') {
        e.preventDefault()
        if (menuFor !== null) {
          setMenuFor(null)
          return
        }
        if (deleteConfirmFor !== null) {
          setDeleteConfirmFor(null)
          return
        }
        if (newCollectionInput !== null) {
          setNewCollectionInput(null)
          return
        }
        onClose()
        return
      }
      if (e.key === 'Tab') {
        // Focus trap (S21): cycle within the dialog root instead of leaking
        // focus out to whatever's behind the modal. Applies regardless of
        // menu/delete-confirm state — those are themselves inside
        // `dialogRef`'s subtree (the ⋯ menu renders as the dialog's last
        // child), so they're already covered by the same query.
        const root = dialogRef.current
        if (!root) return
        const focusable = Array.from(
          root.querySelectorAll<HTMLElement>(
            'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
          ),
        ).filter((el) => el.offsetParent !== null)
        if (focusable.length === 0) return
        const first = focusable[0]
        const last = focusable[focusable.length - 1]
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault()
          last.focus()
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault()
          first.focus()
        }
        return
      }
      if (menuFor !== null) return
      const target = e.target as HTMLElement | null
      const isTextInput =
        target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target?.isContentEditable === true
      if (isTextInput) return
      if (e.key === 'ArrowRight') {
        e.preventDefault()
        moveSelection(1)
      } else if (e.key === 'ArrowLeft') {
        e.preventDefault()
        moveSelection(-1)
      } else if (e.key === 'ArrowDown') {
        e.preventDefault()
        // Measured fresh on every keydown rather than cached in state — the
        // grid's responsive `auto-fill` track can change column count on a
        // window resize the dialog never gets a dedicated event for, and a
        // live DOM read is always correct without an extra ResizeObserver.
        moveSelection(measureGridColumns(gridInnerRef.current))
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        moveSelection(-measureGridColumns(gridInnerRef.current))
      } else if (e.key === 'Enter') {
        // A focused control (the armed delete-confirm's Delete/Cancel
        // button, a keyword chip's remove ×, the Collection <select>, a tile's
        // own Insert/⋯ buttons, …) owns Enter on itself — the category-default
        // action below is a document-level fallback for when nothing more
        // specific has focus, not a hijack of every focused button. Input/
        // textarea/contentEditable are already excluded above via
        // `isTextInput` (TS narrows `target` accordingly), so only
        // button/select need checking here.
        const isControlTarget = target instanceof HTMLButtonElement || target instanceof HTMLSelectElement
        if (isControlTarget) return
        const item = displayItems.find((i) => i.file.fileName === selectedFileName)
        if (!item || item.error) return
        e.preventDefault()
        if (e.metaKey || e.ctrlKey) runOtherAction(item)
        else runDefaultAction(item)
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  })

  if (!open) return null

  const canReveal = libraryStore().capabilities().canReveal
  const canDownload = libraryStore().capabilities().canDownload

  /** The search field — the header's only surviving row for the `'modal'`
   * variant (finding #1), and the sidebar's TOP row for `'window'` (finding
   * #2). Written once so the two placements can never drift (same ref, same
   * handlers), and rendered at exactly one of the two call sites below per
   * render — never both at once. */
  function renderSearch() {
    return (
      <div className="hwlib__search">
        <SearchGlyph />
        <input
          ref={searchInputRef}
          type="text"
          placeholder="Search name or keyword…"
          aria-label="Search the library"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
    )
  }

  function openMenuFor(item: LibraryItem, anchor: HTMLElement) {
    if (variant === 'window') {
      void openNativeMenu(item)
      return
    }
    const rect = anchor.getBoundingClientRect()
    setMenuFor({
      fileName: item.file.fileName,
      anchor: { top: rect.top, left: rect.left, bottom: rect.bottom },
    })
  }

  // 'window' hosting: the surrounding NATIVE window provides the chrome —
  // no scrim, no fixed size, no aria-modal (nothing else exists in the
  // document to be modal against), and no fake titlebar row either: the
  // "LIBRARY" label and ✕ close button below are `'modal'`-only (finding
  // #1) — the real titlebar already supplies both. The body underneath is
  // three full-height columns (sidebar / center / detail) either way
  // (finding #2); the header, when it exists at all, sits above them.
  const panel = (
      <div
        className={variant === 'window' ? 'hwlib hwlib--window' : 'hwlib'}
        ref={dialogRef}
        onClick={variant === 'modal' ? (e) => e.stopPropagation() : undefined}
        role={variant === 'modal' ? 'dialog' : 'region'}
        aria-modal={variant === 'modal' ? 'true' : undefined}
        aria-label="Library"
      >
        {variant === 'modal' && (
          <header className="hwlib__header">
            <span className="hwlib__label">Library</span>
            {renderSearch()}
            <button type="button" className="hwlib__close" aria-label="Close" onClick={onClose}>
              <CloseGlyph />
            </button>
          </header>
        )}

        <div className="hwlib__body" ref={bodyRef}>
          <aside className="hwlib__sidebar" style={{ flex: `0 0 ${colWidths.sidebar}px`, width: colWidths.sidebar }}>
            {variant === 'window' && <div className="hwlib__sidebar-search">{renderSearch()}</div>}

            <div className="hwlib__section-label">Categories</div>
            <div className="hwlib__cat-list" role="tablist" aria-label="Category">
              {CATEGORIES.map((c) => (
                <button
                  key={c.value}
                  type="button"
                  role="tab"
                  aria-selected={category === c.value}
                  className={`hwlib__cat-row${category === c.value ? ' hwlib__cat-row--active' : ''}`}
                  onClick={() => setCategory(c.value)}
                >
                  <span>{c.label}</span>
                  <span className="hwlib__cat-count">{categoryCounts[c.value]}</span>
                </button>
              ))}
            </div>

            <div className="hwlib__section-label">Collections</div>
            <div className="hwlib__collections">
              <button
                type="button"
                className={`hwlib__collection-row${selectedCollection === null ? ' hwlib__collection-row--active' : ''}`}
                onClick={() => setSelectedCollection(null)}
              >
                All
              </button>
              {allCollections.map((c) => (
                <button
                  key={c.path}
                  type="button"
                  className={`hwlib__collection-row${selectedCollection === c.path ? ' hwlib__collection-row--active' : ''}`}
                  style={{ paddingLeft: `${8 + c.depth * 14}px` }}
                  onClick={() => setSelectedCollection(c.path)}
                >
                  {c.label}
                </button>
              ))}
              {newCollectionInput === null ? (
                <button type="button" className="hwlib__new-collection" onClick={() => setNewCollectionInput('')}>
                  + New collection
                </button>
              ) : (
                <input
                  className="hwlib__new-collection-input"
                  autoFocus
                  aria-label="New collection name"
                  value={newCollectionInput}
                  onChange={(e) => setNewCollectionInput(e.target.value)}
                  onBlur={commitNewCollection}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault()
                      commitNewCollection()
                    } else if (e.key === 'Escape') {
                      e.preventDefault()
                      e.stopPropagation()
                      setNewCollectionInput(null)
                    }
                  }}
                />
              )}
            </div>
          </aside>

          {/* Finder-style resizable divider (finding #1): pointer-capture
              drag, same pattern as LibraryListHead's column resize. */}
          <span
            className="hwlib__col-resize"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize sidebar"
            onPointerDown={(e) => beginColResize('sidebar', e)}
          />

          {/* Center column: the scope chips + grid/list toggle sit in a
              slim bar ABOVE the viewport (finding #2) — shared by both
              variants, so this is the one piece of body structure that
              never differs between them. */}
          <div className="hwlib__center">
            <div className="hwlib__center-bar">
              <div className="hwlib__scopes" role="radiogroup" aria-label="Scope">
                {SCOPES.map((s) => (
                  <button
                    key={s.value}
                    type="button"
                    className={`hwlib__chip${scope === s.value ? ' hwlib__chip--active' : ''}`}
                    aria-pressed={scope === s.value}
                    onClick={() => setScope(s.value)}
                  >
                    {s.label}
                  </button>
                ))}
              </div>
              <div className="hwlib__view-toggle" role="group" aria-label="View">
                <button
                  type="button"
                  className={`hwlib__view-btn${view === 'grid' ? ' hwlib__view-btn--active' : ''}`}
                  aria-label="Grid view"
                  aria-pressed={view === 'grid'}
                  onClick={() => setView('grid')}
                >
                  <GridViewGlyph />
                </button>
                <button
                  type="button"
                  className={`hwlib__view-btn${view === 'list' ? ' hwlib__view-btn--active' : ''}`}
                  aria-label="List view"
                  aria-pressed={view === 'list'}
                  onClick={() => setView('list')}
                >
                  <ListViewGlyph />
                </button>
              </div>
            </div>

            <div
              className="hwlib__grid"
              ref={gridRef}
              // List view is a real three-column structure: role=grid with
              // row/columnheader/gridcell children (the only tree in which
              // aria-sort is validly exposed). The thumbnail view stays a
              // flat listbox of options.
              role={view === 'list' ? 'grid' : 'listbox'}
              aria-label={`${CATEGORY_LABEL[category]} items`}
              tabIndex={0}
              aria-activedescendant={selectedItem ? tileElementId(selectedItem.file.fileName) : undefined}
            >
              {unavailable ? (
                <div className="hwlib__empty">The library isn&rsquo;t available in this browser.</div>
              ) : loading ? (
                <div className="hwlib__empty">Loading…</div>
              ) : needsReconnect ? (
                <div className="hwlib__empty">
                  <div>Your library folder needs permission again.</div>
                  <button
                    type="button"
                    className="hwlib__btn-secondary"
                    style={{ marginTop: '10px' }}
                    onClick={() => {
                      void libraryStore()
                        .reconnect?.()
                        .then((ok) => {
                          if (ok) {
                            setReconnectDenied(false)
                            setReloadToken((t) => t + 1)
                          } else {
                            setReconnectDenied(true)
                          }
                        })
                    }}
                  >
                    Reconnect
                  </button>
                  {reconnectDenied && (
                    <div style={{ marginTop: '10px' }}>
                      The browser denied access. Pick the folder again under Settings ▸ Folders, or
                      switch back to browser storage there.
                    </div>
                  )}
                </div>
              ) : loadError !== null ? (
                <div className="hwlib__empty">Couldn&rsquo;t read the library: {loadError}</div>
              ) : items.length === 0 ? (
                <div className="hwlib__empty">Save a selection with &ldquo;Save to Library&rdquo; to get started.</div>
              ) : displayItems.length === 0 ? (
                <div className="hwlib__empty">
                  {query !== '' ? 'No items match your search.' : `No ${CATEGORY_LABEL_LOWER[category]} here yet.`}
                </div>
              ) : view === 'list' ? (
                <>
                  <LibraryListHead
                    widths={listColWidths}
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                    onSort={handleSort}
                    onResize={handleResizeCol}
                  />
                  <div className="hwlib__list" ref={gridInnerRef}>
                    {displayItems.map((item) => (
                      <LibraryListRow
                        key={item.file.fileName}
                        item={item}
                        thumbUrl={thumbUrlsRef.current.get(item.contentHash) ?? null}
                        textureUrl={textureUrlsRef.current.get(item.file.fileName) ?? null}
                        selected={item.file.fileName === selectedFileName}
                        placementCount={placementCountOf(item)}
                        inPalette={inPaletteMap[item.file.fileName] ?? false}
                        widths={listColWidths}
                        onSelect={() => {
                          setSelectedFileName(item.file.fileName)
                          gridRef.current?.focus()
                        }}
                        onActivateDefault={() => runDefaultAction(item)}
                        onOpenMenu={(anchor) => openMenuFor(item, anchor)}
                      />
                    ))}
                  </div>
                </>
              ) : (
                <div className={`hwlib__grid-inner hwlib__grid-inner--${category}`} ref={gridInnerRef}>
                  {displayItems.map((item) => (
                    <LibraryTile
                      key={item.file.fileName}
                      item={item}
                      thumbUrl={thumbUrlsRef.current.get(item.contentHash) ?? null}
                      textureUrl={textureUrlsRef.current.get(item.file.fileName) ?? null}
                      selected={item.file.fileName === selectedFileName}
                      placementCount={placementCountOf(item)}
                      inPalette={inPaletteMap[item.file.fileName] ?? false}
                      defaultActionLabel={defaultActionLabel(item)}
                      onSelect={() => {
                        setSelectedFileName(item.file.fileName)
                        // Keep DOM focus on the grid so arrow-key nav keeps
                        // working right after a mouse click, matching what
                        // `aria-activedescendant` claims (S21).
                        gridRef.current?.focus()
                      }}
                      onActivateDefault={() => runDefaultAction(item)}
                      onOpenMenu={(anchor) => openMenuFor(item, anchor)}
                    />
                  ))}
                </div>
              )}
            </div>
          </div>

          {/* Finder-style resizable divider (finding #1) — sits on the
              detail pane's own LEFT edge, so `beginColResize` flips the
              drag sign for this one (see its doc comment). */}
          <span
            className="hwlib__col-resize"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize detail pane"
            onPointerDown={(e) => beginColResize('detail', e)}
          />

          <LibraryDetailPane
            item={selectedItem}
            thumbUrl={selectedItem ? (thumbUrlsRef.current.get(selectedItem.contentHash) ?? null) : null}
            textureUrl={selectedItem ? (textureUrlsRef.current.get(selectedItem.file.fileName) ?? null) : null}
            width={colWidths.detail}
            native={variant === 'window'}
            collections={allCollections}
            placementCount={placementCountOf(selectedItem)}
            canReveal={canReveal}
            canDownload={canDownload}
            inPalette={selectedItem ? (inPaletteMap[selectedItem.file.fileName] ?? false) : false}
            nowMs={nowMs}
            actionError={actionError}
            deleteArmed={selectedItem !== null && deleteConfirmFor === selectedItem.file.fileName}
            onRename={(name) => selectedItem && handleRename(selectedItem, name)}
            onAddKeyword={(kw) => selectedItem && handleAddKeyword(selectedItem, kw)}
            onRemoveKeyword={(kw) => selectedItem && handleRemoveKeyword(selectedItem, kw)}
            onChangeCollection={(c) => selectedItem && handleChangeCollection(selectedItem, c)}
            onRemoveSourceInfo={() => selectedItem && handleRemoveSourceInfo(selectedItem)}
            onPrimaryAction={() => selectedItem && runDefaultAction(selectedItem)}
            onSecondaryAction={() => selectedItem && runOtherAction(selectedItem)}
            onOpenAsDocument={() => selectedItem && handleOpenAsDocument(selectedItem)}
            onReRenderThumbnail={() => selectedItem && void handleRerenderThumbnail(selectedItem)}
            onReveal={() => selectedItem && void handleReveal(selectedItem)}
            onDownload={() => selectedItem && handleDownload(selectedItem)}
            onRequestDelete={() => selectedItem && void requestDelete(selectedItem)}
            onConfirmDelete={() => selectedItem && void handleDelete(selectedItem)}
            onCancelDelete={() => setDeleteConfirmFor(null)}
          />
        </div>

        <footer className="hwlib__footer">
          <div className="hwlib__hints">
            <kbd>↵</kbd> {selectedItem ? defaultActionLabel(selectedItem) : 'Insert'} · <kbd>⌘↵</kbd>{' '}
            {selectedItem ? otherActionLabel(selectedItem) : 'Open'} · <kbd>Esc</kbd> Close
          </div>
          <div className="hwlib__footer-path">
            {!unavailable && (folderPath !== null ? `${folderPath} · ${itemCountLabel(items.length)}` : itemCountLabel(items.length))}
          </div>
        </footer>

        {menuFor !== null && menuItem !== null && (
          <LibraryMenu
            itemName={menuItem.displayName}
            anchor={menuFor.anchor}
            errored={menuItem.error !== undefined}
            canReveal={canReveal}
            canDownload={canDownload}
            hasSourceInfo={menuItem.category !== 'model' && menuItem.meta.sourceDoc !== undefined}
            onClose={() => setMenuFor(null)}
            onOpenAsDocument={() => handleOpenAsDocument(menuItem)}
            onAddToCollection={() => setSelectedFileName(menuItem.file.fileName)}
            onRename={() => setSelectedFileName(menuItem.file.fileName)}
            onRerenderThumbnail={() => void handleRerenderThumbnail(menuItem)}
            onRemoveSourceInfo={() => handleRemoveSourceInfo(menuItem)}
            onReveal={() => void handleReveal(menuItem)}
            onDownload={() => handleDownload(menuItem)}
            onDeleteRequest={() => {
              setSelectedFileName(menuItem.file.fileName)
              setDeleteConfirmFor(menuItem.file.fileName)
            }}
          />
        )}
      </div>
  )

  if (variant === 'window') {
    return (
      <>
        <style>{LIBRARY_CSS}</style>
        {panel}
      </>
    )
  }
  return (
    <div className="hwlib-overlay" onClick={onClose}>
      <style>{LIBRARY_CSS}</style>
      {panel}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Styles — a scoped stylesheet over design tokens (WelcomeScreen's
// convention), so tiles/menus get real :hover/:focus-visible states and the
// whole layout reads in one place. Every color/font resolves through
// theme/tokens.css; both themes are covered by construction (no literal
// hex here besides the checkerboard, which is a neutral mid-gray by design
// so translucency reads the same in both themes).
// ---------------------------------------------------------------------------

const LIBRARY_CSS = `
.hwlib-overlay {
  position: fixed;
  inset: 0;
  background: var(--backdrop-dim);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 2000;
}
.hwlib--window {
  width: 100vw !important;
  height: 100vh !important;
  border: none !important;
  border-radius: 0 !important;
  box-shadow: none !important;
}
.hwlib {
  display: flex;
  flex-direction: column;
  width: min(920px, 92vw);
  height: min(632px, 86vh);
  background: var(--surface-window);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-palette);
  box-shadow: var(--shadow-window);
  font-family: var(--font-family-ui);
  color: var(--text-secondary);
  overflow: hidden;
}

/* ---- Header ------------------------------------------------------------ */
.hwlib__header {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border-hairline);
  flex: 0 0 auto;
}
.hwlib__label {
  font-family: var(--font-family-mono);
  font-size: var(--font-size-section-header);
  font-weight: 600;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--text-section);
}
.hwlib__search {
  /* border-box (finding: the sidebar placement below sets width:100% —
     under the default content-box model that's 100% of the sidebar's
     content width PLUS this padding/border on top, overflowing the
     sidebar horizontally by ~20px and forcing a scrollbar it doesn't need.
     border-box makes width:100% actually mean 100%, same as the fixed
     250px header placement was always visually implying anyway.) */
  box-sizing: border-box;
  display: flex;
  align-items: center;
  gap: 6px;
  width: 250px;
  padding: 6px 9px;
  background: var(--surface-input);
  border: 1px solid var(--border-panel);
  border-radius: var(--radius-control);
}
/* Sidebar placement (the 'window' variant, finding #2): a fixed 250px
   search box doesn't fit the 168px-wide sidebar — it fills the sidebar's
   own width instead, like Hew's own tool-rail search field. */
.hwlib__sidebar-search {
  padding: 0 0 8px;
}
.hwlib__sidebar-search .hwlib__search {
  width: 100%;
}
.hwlib__search svg {
  width: 13px;
  height: 13px;
  color: var(--text-faint);
  flex: 0 0 auto;
}
.hwlib__search input {
  flex: 1;
  min-width: 0;
  background: none;
  border: none;
  outline: none;
  color: var(--text-primary);
  font-family: var(--font-family-ui);
  font-size: 12.5px;
}
.hwlib__scopes {
  display: flex;
  gap: 6px;
}
.hwlib__chip {
  padding: 5px 12px;
  border-radius: 999px;
  border: 1px solid var(--border-panel);
  background: none;
  color: var(--text-muted);
  font-family: var(--font-family-ui);
  font-size: 11.5px;
  cursor: pointer;
}
.hwlib__chip--active {
  background: var(--accent-base);
  border-color: var(--accent-base);
  color: #fff;
}
.hwlib__view-toggle {
  display: flex;
  gap: 2px;
  padding: 2px;
  border-radius: var(--radius-control);
  background: var(--surface-input);
  border: 1px solid var(--border-panel);
}
.hwlib__view-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 22px;
  background: none;
  border: none;
  border-radius: calc(var(--radius-control) - 2px);
  color: var(--text-muted);
  cursor: pointer;
}
.hwlib__view-btn svg {
  width: 13px;
  height: 13px;
}
.hwlib__view-btn:hover {
  color: var(--text-primary);
}
.hwlib__view-btn--active {
  background: var(--accent-base);
  color: #fff;
}
.hwlib__close {
  margin-left: auto;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  background: none;
  border: none;
  border-radius: var(--radius-control);
  color: var(--text-muted);
  cursor: pointer;
}
.hwlib__close:hover {
  background: var(--accent-tint-15);
  color: var(--text-primary);
}
.hwlib__close svg {
  width: 13px;
  height: 13px;
}

/* ---- Body layout --------------------------------------------------------*/
.hwlib__body {
  display: flex;
  flex: 1 1 auto;
  min-height: 0;
}

/* ---- Center column (finding #2) ------------------------------------------
   The scope chips + grid/list toggle live in a slim bar above the viewport,
   identically for both variants — this is the structure the two hosting
   modes share; only the header above it (modal-only) and the search box's
   placement (sidebar vs. header) differ. */
.hwlib__center {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
}
.hwlib__center-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border-hairline);
  flex: 0 0 auto;
}

/* ---- Sidebar --------------------------------------------------------- */
.hwlib__sidebar {
  /* border-box (finding #1): the width below is now driven by resizable
     state (LibraryDialog's inline style) — border-box makes that number
     the sidebar's TRUE outer width, padding and border included, instead of
     content-box silently adding ~20px on top of it (the same class of bug
     .hwlib__search's doc comment above describes). */
  box-sizing: border-box;
  flex: 0 0 168px;
  /* Static half of finding #1's self-healing fix: caps this column at 40%
     of .hwlib__body's own width no matter what the inline width style
     (resizable state, itself already clamped — see LibraryDialog's
     loadColWidths/clampColWidthLive) says. This alone guarantees the
     center column and BOTH resize dividers stay visible and reachable at
     any window size, even from a hand-edited localStorage value or a very
     narrow window — no in-UI recovery needed, because the layout can never
     actually clip them in the first place. */
  max-width: 40%;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 14px 10px;
  border-right: 1px solid var(--border-hairline);
  overflow-y: auto;
}
/* The divider between two body columns (finding #1) — a real flex sibling
   rather than an absolutely-positioned overlay, so it can never end up
   clipped by a neighboring column's own overflow-y: auto (the sidebar's
   overflow, notably, per the CSS "co-dependency" rule that forces
   overflow-x to 'auto' too whenever only overflow-y is set explicitly). */
.hwlib__col-resize {
  flex: 0 0 5px;
  align-self: stretch;
  cursor: col-resize;
  touch-action: none;
}
.hwlib__col-resize:hover {
  background: var(--accent-tint-15);
}
.hwlib__section-label {
  font-family: var(--font-family-mono);
  font-size: var(--font-size-section-header);
  font-weight: 600;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--text-section);
  padding: 6px 6px 2px;
}
.hwlib__cat-list, .hwlib__collections {
  display: flex;
  flex-direction: column;
  gap: 1px;
  margin-bottom: 8px;
}
.hwlib__cat-row, .hwlib__collection-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  padding: 6px 8px;
  background: none;
  border: none;
  border-radius: var(--radius-panel-item);
  color: var(--text-secondary);
  font-family: var(--font-family-ui);
  font-size: var(--font-size-tool-row);
  text-align: left;
  cursor: pointer;
}
.hwlib__cat-row:hover, .hwlib__collection-row:hover {
  background: var(--accent-tint-15);
}
.hwlib__cat-row--active, .hwlib__collection-row--active {
  background: var(--accent-tint-18);
  color: var(--accent-text-on-tint);
  font-weight: 600;
}
.hwlib__cat-count {
  font-family: var(--font-family-mono);
  font-size: 11px;
  color: var(--text-faint);
}
.hwlib__new-collection {
  padding: 6px 8px;
  background: none;
  border: none;
  border-radius: var(--radius-panel-item);
  color: var(--text-faint);
  font-family: var(--font-family-ui);
  font-size: 12px;
  text-align: left;
  cursor: pointer;
}
.hwlib__new-collection:hover {
  color: var(--accent-base);
}
.hwlib__new-collection-input {
  margin: 0 8px;
  padding: 5px 7px;
  background: var(--surface-input);
  border: 1px solid var(--accent-border);
  border-radius: var(--radius-control);
  color: var(--text-primary);
  font-family: var(--font-family-ui);
  font-size: 12px;
  outline: none;
}

/* ---- Grid ---------------------------------------------------------------*/
.hwlib__grid {
  flex: 1 1 auto;
  min-width: 0;
  overflow-y: auto;
  padding: 14px;
}
.hwlib__empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text-faint);
  font-size: 12.5px;
  text-align: center;
  padding: 20px;
}
/* auto-fill + a minmax floor: the grid always wraps to fit the available
   width (never a horizontal scrollbar) instead of assuming a fixed 3/4-
   column count — the keyboard nav measures whatever this actually renders
   (see library/gridMeasure.ts) rather than assuming it back. */
.hwlib__grid-inner {
  display: grid;
  gap: 14px;
}
/* 'all' (finding #4) mixes material swatch tiles in with thumbnail tiles —
   it uses the LARGER of the two tracks (matching component/model) so a
   thumbnail tile never gets squeezed into a track sized for the smaller
   material swatch. */
.hwlib__grid-inner--component, .hwlib__grid-inner--model, .hwlib__grid-inner--all {
  grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
}
.hwlib__grid-inner--material {
  grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
}

/* ---- Tiles ---------------------------------------------------------------*/
.hwlib__tile {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0; /* a long name's intrinsic min-content must never stretch
                   the grid column past its 1fr share — the uniform-tile-
                   size finding's actual root cause. */
  border-radius: var(--radius-panel-item);
  cursor: pointer;
  outline: none;
}
.hwlib__tile-thumb, .hwlib__tile-swatch {
  position: relative;
  height: 118px;
  border-radius: var(--radius-panel-item);
  background: var(--surface-panel);
  border: 2px solid transparent;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-faint);
  overflow: hidden;
}
.hwlib__tile--material .hwlib__tile-swatch {
  height: 92px;
}
.hwlib__tile--selected .hwlib__tile-thumb,
.hwlib__tile--selected .hwlib__tile-swatch {
  border-color: var(--accent-base);
}
.hwlib__tile-thumb svg {
  width: 30px;
  height: 30px;
}
.hwlib__tile-thumb-img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.hwlib__tile-swatch-fill {
  position: absolute;
  inset: 0;
}
.hwlib__tile-error-fill {
  color: var(--status-warning);
}
.hwlib__tile--error .hwlib__tile-thumb, .hwlib__tile--error .hwlib__tile-swatch {
  color: var(--status-warning);
}
.hwlib__tile-dot {
  position: absolute;
  top: 6px;
  left: 6px;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--accent-base);
  z-index: 1;
}
.hwlib__tile-actions {
  position: absolute;
  top: 6px;
  right: 6px;
  display: flex;
  gap: 4px;
  opacity: 0;
  transition: opacity 120ms ease;
}
.hwlib__tile:hover .hwlib__tile-actions,
.hwlib__tile:focus-within .hwlib__tile-actions {
  opacity: 1;
}
.hwlib__tile-action-btn {
  padding: 4px 10px;
  border-radius: var(--radius-control);
  border: none;
  background: var(--accent-base);
  color: #fff;
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
}
.hwlib__tile-menu-btn {
  width: 22px;
  height: 22px;
  border-radius: var(--radius-control);
  border: none;
  background: var(--surface-overlay);
  color: var(--text-primary);
  cursor: pointer;
  line-height: 1;
}
.hwlib__tile-name {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--text-primary);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  white-space: normal;
  word-break: break-word;
}
.hwlib__tile-sub {
  font-size: 10.5px;
  color: var(--text-faint);
  min-height: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Checkerboard base for translucent material swatches — neutral mid-gray so
   partial opacity reads identically in both themes. */
.hwlib__checker {
  background-image:
    linear-gradient(45deg, rgba(128,128,128,0.28) 25%, transparent 25%),
    linear-gradient(-45deg, rgba(128,128,128,0.28) 25%, transparent 25%),
    linear-gradient(45deg, transparent 75%, rgba(128,128,128,0.28) 75%),
    linear-gradient(-45deg, transparent 75%, rgba(128,128,128,0.28) 75%);
  background-size: 12px 12px;
  background-position: 0 0, 0 6px, 6px -6px, -6px 0px;
}

/* ---- List view (finding #3) -----------------------------------------------
   A real Finder/Explorer-style list: exactly Name/Type/Size, each sortable
   and resizable. .hwlib__list-head and every .hwlib__row's three cells
   share the SAME widths (LibraryListColWidths, passed down from
   LibraryDialog) so the header and body columns always line up. */
.hwlib__list-head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 2px 8px 8px;
  margin-bottom: 4px;
  border-bottom: 1px solid var(--border-hairline);
}
.hwlib__list-head-cell {
  position: relative;
  display: flex;
  align-items: center;
  min-width: 0;
}
.hwlib__list-sort-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 2px 0;
  background: none;
  border: none;
  color: var(--text-faint);
  font-family: var(--font-family-mono);
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  cursor: pointer;
}
.hwlib__list-sort-btn:hover {
  color: var(--text-primary);
}
.hwlib__list-head-cell[aria-sort="ascending"] .hwlib__list-sort-btn,
.hwlib__list-head-cell[aria-sort="descending"] .hwlib__list-sort-btn {
  color: var(--text-primary);
}
.hwlib__list-sort-glyph {
  font-size: 8px;
}
/* The resize handle sits on the column's own right edge and drags THAT
   column's width — the standard file-manager convention. Only Name and Type
   get one; Size is the flexible remainder, so it still visibly resizes as a
   result of the other two even though it has no handle of its own. */
.hwlib__list-col-resize {
  position: absolute;
  top: -2px;
  right: -6px;
  bottom: -2px;
  width: 10px;
  cursor: col-resize;
  touch-action: none;
}
.hwlib__list-col-resize:hover {
  background: var(--accent-tint-15);
}
.hwlib__list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.hwlib__row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 8px;
  border-radius: var(--radius-panel-item);
  cursor: pointer;
  outline: none;
  border: 1px solid transparent;
}
.hwlib__row:hover {
  background: var(--accent-tint-15);
}
.hwlib__row--selected {
  border-color: var(--accent-base);
  background: var(--accent-tint-15);
}
.hwlib__row--error {
  color: var(--status-warning);
}
.hwlib__row-cell {
  display: flex;
  align-items: center;
  min-width: 0;
}
.hwlib__row-cell--type, .hwlib__row-cell--size {
  color: var(--text-faint);
  font-family: var(--font-family-mono);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hwlib__row-cell--name {
  gap: 8px;
}
.hwlib__row-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent-base);
  flex: 0 0 auto;
}
.hwlib__row-thumb {
  position: relative;
  flex: 0 0 auto;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-control);
  background: var(--surface-panel);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-faint);
  overflow: hidden;
}
.hwlib__row-thumb svg {
  width: 16px;
  height: 16px;
}
.hwlib__row-thumb-img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.hwlib__row-name {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 12.5px;
  font-weight: 500;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hwlib__row-menu-btn {
  flex: 0 0 auto;
  margin-left: auto;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-control);
  border: none;
  background: none;
  color: var(--text-muted);
  cursor: pointer;
  line-height: 1;
  opacity: 0;
}
.hwlib__row:hover .hwlib__row-menu-btn,
.hwlib__row:focus-within .hwlib__row-menu-btn {
  opacity: 1;
}

/* ---- Detail pane ---------------------------------------------------------*/
.hwlib__detail {
  /* border-box (finding #1) — same reasoning as .hwlib__sidebar's: width
     is resizable state now, and must mean the pane's true outer width. */
  box-sizing: border-box;
  flex: 0 0 236px;
  /* Same self-healing cap as .hwlib__sidebar's, and for the same reason —
     see its comment. */
  max-width: 40%;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 16px;
  border-left: 1px solid var(--border-hairline);
  overflow-y: auto;
}
.hwlib__detail--empty {
  align-items: center;
  justify-content: center;
  color: var(--text-faint);
  font-size: 12px;
  text-align: center;
}
.hwlib__detail-thumb {
  position: relative;
  height: 150px;
  border-radius: var(--radius-panel-item);
  background: var(--surface-panel);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-faint);
  overflow: hidden;
}
.hwlib__detail-thumb--error {
  color: var(--status-warning);
}
.hwlib__detail-thumb svg {
  width: 40px;
  height: 40px;
}
.hwlib__detail-thumb-img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.hwlib__detail-swatch {
  position: absolute;
  inset: 0;
}
.hwlib__detail-swatch-fill {
  position: absolute;
  inset: 0;
}
.hwlib__detail-rerender {
  position: absolute;
  bottom: 6px;
  right: 6px;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border-radius: var(--radius-control);
  border: 1px solid var(--border-panel);
  background: var(--surface-overlay);
  color: var(--text-secondary);
  font-size: 10.5px;
  cursor: pointer;
}
.hwlib__detail-rerender svg {
  width: 11px;
  height: 11px;
}
.hwlib__detail-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}
.hwlib__name-input {
  padding: 5px 7px;
  background: var(--surface-input);
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  color: var(--text-primary);
  font-family: var(--font-family-ui);
  font-size: 14px;
  font-weight: 600;
  outline: none;
  /* Auto-growing textarea (finding: show the FULL name, wrapped, never
     truncated) — height is set imperatively from scrollHeight on every
     draft change; resize:none keeps the user from fighting that. */
  resize: none;
  overflow: hidden;
  white-space: pre-wrap;
  word-break: break-word;
}
.hwlib__name-input:focus {
  border-color: var(--accent-border);
}
.hwlib__detail-error {
  font-size: 12px;
  color: var(--status-leaky);
}
.hwlib__keywords {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  align-items: center;
}
.hwlib__keyword-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: 999px;
  background: var(--surface-input);
  border: 1px solid var(--border-panel);
  color: var(--text-secondary);
  font-size: 11px;
}
.hwlib__keyword-chip button {
  background: none;
  border: none;
  color: var(--text-faint);
  cursor: pointer;
  font-size: 11px;
  line-height: 1;
  padding: 0;
}
.hwlib__keyword-add {
  padding: 3px 9px;
  border-radius: 999px;
  border: 1px dashed var(--border-panel);
  background: none;
  color: var(--text-faint);
  font-size: 11px;
  cursor: pointer;
}
.hwlib__keyword-input {
  padding: 3px 8px;
  border-radius: 999px;
  border: 1px solid var(--accent-border);
  background: var(--surface-input);
  color: var(--text-primary);
  font-size: 11px;
  outline: none;
  width: 90px;
}
.hwlib__field-row {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 11px;
  color: var(--text-muted);
}
.hwlib__select {
  padding: 5px 7px;
  background: var(--surface-input);
  border: 1px solid var(--border-panel);
  border-radius: var(--radius-control);
  color: var(--text-primary);
  font-family: var(--font-family-ui);
  font-size: 12px;
  cursor: pointer;
}
.hwlib__meta {
  font-family: var(--font-family-mono);
  font-size: 10.5px;
  line-height: 1.6;
  color: var(--text-faint);
}
.hwlib__source-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}
.hwlib__source-row span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hwlib__source-row .hwlib__btn-link {
  flex: 0 0 auto;
  font-family: var(--font-family-ui);
  font-size: 10px;
  padding: 0;
}
.hwlib__inmodel {
  font-size: 11px;
  font-weight: 500;
  color: var(--accent-text-on-tint);
}
.hwlib__actions {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: auto;
  padding-top: 8px;
}
.hwlib__btn-primary, .hwlib__btn-secondary, .hwlib__btn-danger {
  padding: 8px 12px;
  border-radius: var(--radius-control);
  font-family: var(--font-family-ui);
  font-size: var(--font-size-tool-row);
  font-weight: 600;
  cursor: pointer;
}
.hwlib__btn-primary {
  border: none;
  background: var(--accent-base);
  color: #fff;
}
.hwlib__btn-secondary {
  border: 1px solid var(--border-panel);
  background: var(--surface-input);
  color: var(--text-primary);
}
.hwlib__btn-link {
  border: none;
  background: none;
  color: var(--accent-base);
  font-size: 11.5px;
  text-align: center;
  cursor: pointer;
  padding: 2px;
}
.hwlib__btn-danger {
  border: 1px solid transparent;
  background: none;
  color: var(--danger-base);
}
.hwlib__btn-danger:hover {
  background: color-mix(in srgb, var(--danger-base) 12%, transparent);
}
.hwlib__action-error {
  padding: 6px 8px;
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--danger-base) 10%, transparent);
  color: var(--danger-base);
  font-size: 11px;
}
.hwlib__manage-row {
  display: flex;
  gap: 6px;
  padding-top: 6px;
  border-top: 1px solid var(--border-hairline);
}
.hwlib__manage-row .hwlib__btn-secondary,
.hwlib__manage-row .hwlib__btn-danger {
  flex: 1 1 0;
  text-align: center;
}
/* Native-feel detail buttons (the 'window' variant, finding #5): reset to
   the platform's own push-button chrome (WKWebView's UA stylesheet) instead
   of the custom paint above — only layout survives the reset, restored as
   separate rules below since "all: revert" wipes it along with everything
   else. The modal/web variant never applies this class; there's no native
   look worth chasing on the web. */
.hwlib__native-btn {
  all: revert;
  width: 100%;
  font: caption;
  cursor: pointer;
}
.hwlib__manage-row .hwlib__native-btn {
  flex: 1 1 0;
  width: auto;
}
.hwlib__delete-confirm {
  padding: 10px;
  border-radius: var(--radius-panel-item);
  background: color-mix(in srgb, var(--danger-base) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--danger-base) 30%, transparent);
}
.hwlib__delete-confirm p {
  margin: 0 0 8px;
  font-size: 11.5px;
  color: var(--text-secondary);
}
.hwlib__delete-confirm-actions {
  display: flex;
  gap: 6px;
  justify-content: flex-end;
}

/* ---- Footer ---------------------------------------------------------- */
.hwlib__footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 16px;
  border-top: 1px solid var(--border-hairline);
  flex: 0 0 auto;
}
.hwlib__hints, .hwlib__footer-path {
  font-family: var(--font-family-mono);
  font-size: 10.5px;
  color: var(--text-faint);
}
.hwlib__hints kbd {
  display: inline-block;
  padding: 1px 5px;
  border-radius: var(--radius-kbd);
  background: var(--kbd-bg);
  border: 1px solid var(--kbd-border);
  color: var(--kbd-text);
  font-family: var(--font-family-mono);
  font-size: var(--font-size-kbd);
}

/* ---- ⋯ menu popover ---------------------------------------------------- */
.hwlib__menu-scrim {
  position: fixed;
  inset: 0;
  z-index: 2001;
}
.hwlib__menu {
  position: fixed;
  z-index: 2002;
  min-width: 200px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 1px;
  background: var(--surface-overlay);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-panel-item);
  box-shadow: var(--shadow-palette);
}
.hwlib__menu-item {
  padding: 7px 9px;
  background: none;
  border: none;
  border-radius: var(--radius-control);
  color: var(--text-primary);
  font-family: var(--font-family-ui);
  font-size: 12.5px;
  text-align: left;
  cursor: pointer;
}
.hwlib__menu-item:hover {
  background: var(--accent-tint-15);
}
.hwlib__menu-item--danger {
  color: var(--danger-base);
}
.hwlib__menu-item--danger:hover {
  background: color-mix(in srgb, var(--danger-base) 12%, transparent);
}
.hwlib__menu-divider {
  height: 1px;
  margin: 4px 2px;
  background: var(--border-hairline);
}

@media (prefers-reduced-motion: reduce) {
  .hwlib__tile-actions {
    transition: none;
  }
}
`
