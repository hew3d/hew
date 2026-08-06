/**
 * libraryModel — pure derivation and filtering logic for the Library
 * browser. No I/O, no React: everything here is a plain function over the
 * shapes in `./types`, so it can be unit tested without wasm or a DOM (see
 * `libraryModel.test.ts`, node environment).
 *
 * The single source of truth for an item's curated metadata is its own
 * `hew.library` document attribute dictionary (parsed here from the
 * manifest-only `LibraryItemSummary.doc_attrs`), never a separate database.
 * That dictionary is attacker-shaped (any `.hew` file could be hand-edited
 * or come from an older/newer format version), so every field is read
 * through a type guard — a malformed `hew.library` blob degrades to "no
 * metadata for that field", never a thrown exception.
 */

import type {
  LibraryCategory,
  LibraryFileEntry,
  LibraryItem,
  LibraryItemMeta,
  LibraryItemSummary,
  LibraryMaterialSummary,
} from './types'

// ---------------------------------------------------------------------------
// Meta parsing (type guards over an untrusted doc_attrs blob)
// ---------------------------------------------------------------------------

const VALID_CATEGORIES: readonly LibraryCategory[] = ['component', 'material', 'model']

function isValidCategory(v: unknown): v is LibraryCategory {
  return typeof v === 'string' && (VALID_CATEGORIES as readonly string[]).includes(v)
}

function isStringArray(v: unknown): v is string[] {
  return Array.isArray(v) && v.every((e) => typeof e === 'string')
}

/** Parses a `hew.library` attribute-dictionary value into a `LibraryItemMeta`.
 * Never throws: anything not shaped like an object yields `{}`, and each
 * field is only carried across if it has the right JS type — a bad `id`
 * (say, a number) is dropped rather than poisoning the whole item. */
export function parseLibraryMeta(raw: unknown): LibraryItemMeta {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return {}
  const obj = raw as Record<string, unknown>
  const meta: LibraryItemMeta = {}
  if (typeof obj.id === 'string') meta.id = obj.id
  if (typeof obj.name === 'string') meta.name = obj.name
  if (isValidCategory(obj.category)) meta.category = obj.category
  if (isStringArray(obj.keywords)) meta.keywords = obj.keywords
  if (typeof obj.collection === 'string') meta.collection = obj.collection
  if (typeof obj.savedAt === 'string') meta.savedAt = obj.savedAt
  if (typeof obj.sourceDoc === 'string') meta.sourceDoc = obj.sourceDoc
  return meta
}

/** The bare file name minus any category subfolder (`Components/`,
 * `Materials/`, `Models/`) and its `.hew` extension — the last-resort
 * display name for an item with no metadata and no named definition/root.
 * `fileName` is otherwise treated as an opaque id throughout this module;
 * this is the one place that peels off its directory shape, purely for
 * display. */
function fileStem(fileName: string): string {
  const base = fileName.slice(fileName.lastIndexOf('/') + 1)
  const stem = base.replace(/\.hew$/i, '')
  return stem.length > 0 ? stem : fileName
}

/** Derives a category from manifest shape alone, for items whose metadata
 * doesn't already pin one (`buildLibraryItem`'s `meta.category` always wins
 * over this). Kept deliberately simple and covered by tests rather than
 * exhaustively "correct": a material-only file (no solids, no component
 * definitions, at least one palette entry) is a material; a file holding
 * exactly one component definition and no top-level groups is a component;
 * everything else is a model. */
function deriveCategory(summary: LibraryItemSummary): LibraryCategory {
  const materialOnly = summary.objects === 0 && summary.components === 0 && summary.materials > 0
  if (materialOnly) return 'material'
  const singleDefinition = summary.components === 1 && summary.groups === 0
  if (singleDefinition) return 'component'
  return 'model'
}

/** Builds a full `LibraryItem` from the storage seam's file entry plus the
 * wasm-read manifest summary. Never throws on a well-formed `summary` —
 * callers that fail to even parse a file's manifest should build
 * `erroredItem` instead. */
export function buildLibraryItem(
  file: LibraryFileEntry,
  summary: LibraryItemSummary,
  contentHash: string,
): LibraryItem {
  const meta = parseLibraryMeta(summary.doc_attrs?.['hew.library'])
  const displayName =
    meta.name ?? summary.first_component_name ?? summary.first_root_name ?? fileStem(file.fileName)
  const category = meta.category ?? deriveCategory(summary)
  return { file, summary, meta, displayName, category, contentHash }
}

/** A summary shape with nothing in it — the placeholder `erroredItem` needs
 * to satisfy `LibraryItem.summary`'s required fields without pretending to
 * know anything about a file that failed to parse. */
const EMPTY_SUMMARY: LibraryItemSummary = {
  format_version: 0,
  objects: 0,
  materials: 0,
  components: 0,
  instances: 0,
  groups: 0,
  world_sketches: 0,
  annotations: 0,
  guides: 0,
  first_component_name: null,
  first_component_sid: null,
  first_root_name: null,
  doc_attrs: {},
  material_entries: [],
}

/** Builds the error-state tile for a file that failed to parse (a truncated
 * download, a foreign zip dropped into the folder, an unsupported format
 * version, ...). No silent repair: the browser shows the typed error
 * instead of hiding the file or guessing at its contents. Lists under
 * 'model' by default since there is no manifest to derive a real category
 * from — an arbitrary but harmless bucket for a tile the user will delete
 * or investigate, not insert. */
export function erroredItem(file: LibraryFileEntry, contentHash: string, error: string): LibraryItem {
  return {
    file,
    summary: EMPTY_SUMMARY,
    meta: {},
    displayName: fileStem(file.fileName),
    category: 'model',
    contentHash,
    error,
  }
}

// ---------------------------------------------------------------------------
// Filtering
// ---------------------------------------------------------------------------

export type LibraryScope = 'all' | 'in-model' | 'recent'

export interface FilterOptions {
  query: string
  /** `'all'` skips the category filter entirely (the sidebar's "All" row,
   * finding #4) — every other value filters to that one category exactly,
   * same as before. */
  category: LibraryCategory | 'all'
  scope: LibraryScope
  /** A collection path to additionally filter by (subtree match — see
   * `collectionMatchesSubtree`), or `null` for "any collection" (still
   * subject to category/scope/query). */
  collection: string | null
  /** `hew.library` source id → live placement count in the open document. */
  placements: Record<string, number>
  /** File name → whether that material item's content is already in the
   * open document's palette (the material equivalent of a placement count —
   * materials aren't "placed" as instances, so `placements` alone can't
   * answer "in this model" for them). Absent/omitted keys count as not
   * in-palette, matching how a caller with no `materialInPalette` callback
   * at all (see `LibraryDialog`'s `inPaletteMap`) degrades to "unknown". */
  materialInPalette?: Record<string, boolean>
  nowMs: number
}

// ---------------------------------------------------------------------------
// Collection paths — nested collections mirror how document Tags nest:
// `/`-separated segments, normalized (leading/trailing/double slashes
// collapse away) rather than rejected, so a hand-typed "/Hardware//Screws/"
// means the same thing as "Hardware/Screws".
// ---------------------------------------------------------------------------

/** Splits a raw collection path into trimmed, non-empty segments — the
 * normalization primitive every other collection-path helper builds on. */
export function collectionSegments(raw: string): string[] {
  return raw
    .split('/')
    .map((s) => s.trim())
    .filter((s) => s !== '')
}

/** The canonical form of a collection path: segments rejoined with a single
 * `/`, or `''` for "uncollected" / an all-empty input. */
export function normalizeCollectionPath(raw: string): string {
  return collectionSegments(raw).join('/')
}

/** True when `itemCollection` is `filterPath` itself or lives anywhere in
 * its subtree — matched on whole path SEGMENTS, so selecting "Hardware"
 * matches "Hardware" and "Hardware/Fasteners" but not "HardwareX" (a naive
 * string-prefix check would wrongly match that last case). */
export function collectionMatchesSubtree(itemCollection: string | undefined, filterPath: string): boolean {
  if (itemCollection === undefined) return false
  const filterSegments = collectionSegments(filterPath)
  if (filterSegments.length === 0) return false
  const itemSegments = collectionSegments(itemCollection)
  if (itemSegments.length < filterSegments.length) return false
  return filterSegments.every((seg, i) => itemSegments[i] === seg)
}

/** One row of the sidebar's collection tree: a full path, its own last
 * segment (the label to display, indented by `depth`), and its nesting
 * depth (0 = top-level). */
export interface CollectionNode {
  path: string
  label: string
  depth: number
}

/** Derives the collection tree from a flat list of raw (possibly
 * unnormalized, possibly duplicated) collection paths — dedupes, and
 * synthesizes every intermediate parent so "Hardware/Fasteners" alone still
 * renders a "Hardware" row to nest under, even if no item is collected
 * directly under "Hardware" itself. Pre-order, alphabetical at every level,
 * so callers can render the flat list with `depth`-based indentation. */
export function collectionTreeFromPaths(paths: string[]): CollectionNode[] {
  interface TreeNode {
    label: string
    path: string
    children: Map<string, TreeNode>
  }
  const root = new Map<string, TreeNode>()
  for (const raw of paths) {
    let siblings = root
    let parentPath = ''
    for (const label of collectionSegments(raw)) {
      let node = siblings.get(label)
      if (!node) {
        const path = parentPath === '' ? label : `${parentPath}/${label}`
        node = { label, path, children: new Map() }
        siblings.set(label, node)
      }
      parentPath = node.path
      siblings = node.children
    }
  }
  const result: CollectionNode[] = []
  function walk(siblings: Map<string, TreeNode>, depth: number) {
    const sorted = Array.from(siblings.values()).sort((a, b) => a.label.localeCompare(b.label))
    for (const node of sorted) {
      result.push({ path: node.path, label: node.label, depth })
      walk(node.children, depth + 1)
    }
  }
  walk(root, 0)
  return result
}

const RECENT_WINDOW_MS = 14 * 24 * 60 * 60 * 1000

/** The best timestamp an item has: its own `savedAt` metadata when present
 * and parseable, else the file's last-modified time. `0` means "unknown"
 * (the storage backend can't say — `LibraryFileEntry.mtimeMs`'s documented
 * convention). */
function itemTimestamp(item: LibraryItem): number {
  if (item.meta.savedAt !== undefined) {
    const parsed = Date.parse(item.meta.savedAt)
    if (!Number.isNaN(parsed)) return parsed
  }
  return item.file.mtimeMs
}

/** Rank of a query match against an item, lowest-first (name prefix beats
 * name substring beats a keyword hit); `null` when the item doesn't match
 * at all. */
function matchRank(item: LibraryItem, normalizedQuery: string): number | null {
  const name = item.displayName.toLowerCase()
  if (name.startsWith(normalizedQuery)) return 0
  if (name.includes(normalizedQuery)) return 1
  const keywords = item.meta.keywords ?? []
  if (keywords.some((k) => k.toLowerCase().includes(normalizedQuery))) return 2
  return null
}

/** Filters and (for search/recent) orders items for the browser grid.
 * Category always applies, even with an empty query — the sidebar's
 * category selection is not itself a search. */
export function filterItems(items: LibraryItem[], opts: FilterOptions): LibraryItem[] {
  const { query, category, scope, collection, placements, materialInPalette, nowMs } = opts

  let result = category === 'all' ? items.slice() : items.filter((item) => item.category === category)

  if (collection !== null) {
    result = result.filter((item) => collectionMatchesSubtree(item.meta.collection, collection))
  }

  if (scope === 'in-model') {
    result = result.filter((item) => {
      const id = item.meta.id
      if (id !== undefined && (placements[id] ?? 0) > 0) return true
      // Materials match "in this model" by palette membership, not a
      // placement count — see the `materialInPalette` doc comment above.
      return item.category === 'material' && (materialInPalette?.[item.file.fileName] ?? false)
    })
  } else if (scope === 'recent') {
    result = result
      .filter((item) => {
        const t = itemTimestamp(item)
        return t > 0 && nowMs - t <= RECENT_WINDOW_MS
      })
      .sort((a, b) => itemTimestamp(b) - itemTimestamp(a))
  }

  const normalizedQuery = query.trim().toLowerCase()
  if (normalizedQuery === '') return result

  return result
    .map((item) => ({ item, rank: matchRank(item, normalizedQuery) }))
    .filter((row): row is { item: LibraryItem; rank: number } => row.rank !== null)
    .sort((a, b) => a.rank - b.rank)
    .map((row) => row.item)
}

/** The collection tree in use across `items` — blank/whitespace-only
 * collections don't count (they mean "uncollected", per
 * `LibraryItemMeta.collection`'s doc comment). See `collectionTreeFromPaths`
 * for how intermediate parents get synthesized and the result ordered. */
export function collectionsOf(items: LibraryItem[]): CollectionNode[] {
  const paths: string[] = []
  for (const item of items) {
    const c = item.meta.collection
    if (c !== undefined) paths.push(c)
  }
  return collectionTreeFromPaths(paths)
}

// ---------------------------------------------------------------------------
// List view — sorting (finding #3: a real Name/Type/Size list, sortable)
// ---------------------------------------------------------------------------

const CATEGORY_TYPE_LABEL: Record<LibraryCategory, string> = {
  component: 'Component',
  material: 'Material',
  model: 'Model',
}

/** The List view's Type column text — shared with the sort key below so the
 * two can never drift (sorting "by what's displayed", not some internal
 * enum ordering). */
export function categoryTypeLabel(category: LibraryCategory): string {
  return CATEGORY_TYPE_LABEL[category]
}

export type ListSortColumn = 'name' | 'type' | 'size'
export type ListSortDirection = 'asc' | 'desc'

function listSortKey(item: LibraryItem, column: ListSortColumn): string | number {
  if (column === 'name') return item.displayName.toLowerCase()
  if (column === 'type') return categoryTypeLabel(item.category)
  return item.file.size
}

/** Sorts items for the List view's Name/Type/Size columns. Pure and
 * side-effect-free (never mutates `items`) so the dialog can compose it with
 * `filterItems`'s output as view-local state. A tie always falls back to
 * ascending display-name order — otherwise two same-size or same-category
 * items would have no stable relative order and could visibly jitter across
 * re-renders. */
export function sortListItems(items: LibraryItem[], column: ListSortColumn, direction: ListSortDirection): LibraryItem[] {
  const sign = direction === 'asc' ? 1 : -1
  return [...items].sort((a, b) => {
    const av = listSortKey(a, column)
    const bv = listSortKey(b, column)
    if (av < bv) return -1 * sign
    if (av > bv) return 1 * sign
    return a.displayName.localeCompare(b.displayName)
  })
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/** Byte count as a short, human-scale string: bytes under 1 KB, otherwise
 * KB/MB with one decimal place. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const kb = bytes / 1024
  if (kb < 1024) return `${round1(kb)} KB`
  return `${round1(kb / 1024)} MB`
}

function round1(n: number): string {
  return (Math.round(n * 10) / 10).toFixed(1)
}

function pluralize(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? '' : 's'}`
}

/** The detail pane's mono metadata line, e.g. `4 solids · 2 materials · 1.2
 * MB` — zero-valued counts are omitted (a pure material item has no
 * solids), and the file size is always shown last. */
export function metadataLine(item: LibraryItem): string {
  const parts: string[] = []
  if (item.summary.objects > 0) parts.push(pluralize(item.summary.objects, 'solid'))
  if (item.summary.materials > 0) parts.push(pluralize(item.summary.materials, 'material'))
  parts.push(formatBytes(item.file.size))
  return parts.join(' · ')
}

/** Meters formatted to at most 2 decimal places, trailing zeros trimmed
 * (`0.60` → `0.6`, `1.00` → `1`). */
function formatMeters(m: number): string {
  const rounded = Math.round(m * 100) / 100
  return `${rounded} m`
}

/** A material tile/detail sub-line: `texture · 0.6 m tile`, `color`, or
 * `color · opacity 40%` — opacity only shown when the alpha channel isn't
 * fully opaque (255). */
export function materialSubline(entry: LibraryMaterialSummary): string {
  const parts: string[] = []
  if (entry.texture_asset !== null) {
    parts.push('texture')
    if (entry.texture_world_size !== null) {
      parts.push(`${formatMeters(entry.texture_world_size[0])} tile`)
    }
  } else {
    parts.push('color')
  }
  const alpha = entry.color[3]
  if (alpha !== 255) {
    parts.push(`opacity ${Math.round((alpha / 255) * 100)}%`)
  }
  return parts.join(' · ')
}

function formatAbsoluteDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })
}

/** A relative "Saved …" line for the detail pane: minutes/hours ago for the
 * first day, "yesterday", "N days ago" through 30 days, then an absolute
 * date. Falls back to a neutral "unknown" line when no timestamp is
 * available at all (mtimeMs 0, no savedAt — a backend that can't say). */
export function savedLine(item: LibraryItem, nowMs: number): string {
  const t = itemTimestamp(item)
  if (t <= 0) return 'Saved date unknown'
  const diffMs = Math.max(0, nowMs - t)
  const diffMinutes = Math.floor(diffMs / (60 * 1000))
  const diffHours = Math.floor(diffMs / (60 * 60 * 1000))
  const diffDays = Math.floor(diffMs / (24 * 60 * 60 * 1000))
  if (diffMinutes < 1) return 'Saved just now'
  if (diffHours < 1) return `Saved ${pluralize(diffMinutes, 'minute')} ago`
  if (diffDays < 1) return `Saved ${pluralize(diffHours, 'hour')} ago`
  if (diffDays === 1) return 'Saved yesterday'
  if (diffDays <= 30) return `Saved ${pluralize(diffDays, 'day')} ago`
  return `Saved ${formatAbsoluteDate(t)}`
}
